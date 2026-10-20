use std::{
    collections::VecDeque,
    fs::File,
    io::{self, IsTerminal, Stdout, stdout},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Row, Table, Wrap},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{app::doctor, args::AppArgs};

const FORMATS: &[&str] = &["csv", "txt", "json", "xlsx", "sql", "sqlite", "parquet"];
const TARGETS: &[&str] = &["sqlite", "parquet", "csv"];
const LABELS: [&str; 15] = [
    "Input file",
    "Output name",
    "Source format",
    "Target format",
    "Delimiter",
    "Table name",
    "Encoding override",
    "Batch size",
    "Scan CSV/TXT first",
    "SQLite typed mode",
    "Infer Parquet types",
    "Strict row handling",
    "Drop existing table",
    "First row is data",
    "Start conversion",
];

static EVENT_SINK: OnceLock<Mutex<Option<mpsc::UnboundedSender<UiEvent>>>> = OnceLock::new();
static ERROR_SHOWN: AtomicBool = AtomicBool::new(false);

pub fn error_was_shown() -> bool {
    ERROR_SHOWN.load(Ordering::Relaxed)
}

enum UiEvent {
    Log(String),
    Progress(String),
    Preview(Vec<Vec<String>>, oneshot::Sender<bool>),
}

fn sink() -> &'static Mutex<Option<mpsc::UnboundedSender<UiEvent>>> {
    EVENT_SINK.get_or_init(|| Mutex::new(None))
}

pub fn active() -> bool {
    sink().lock().map(|s| s.is_some()).unwrap_or(false)
}

pub fn log(message: String) {
    send(UiEvent::Log(message));
}
pub fn progress_message(message: String) {
    send(UiEvent::Progress(message));
}

fn send(event: UiEvent) {
    if let Ok(guard) = sink().lock() {
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(event);
        }
    }
}

pub async fn show_preview(rows: Vec<Vec<String>>) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    send(UiEvent::Preview(rows, tx));
    match rx.await {
        Ok(true) => Ok(()),
        Ok(false) => bail!("conversion cancelled at preview"),
        Err(_) => bail!("terminal UI closed before preview confirmation"),
    }
}

#[derive(Default)]
struct Form {
    values: [String; 8],
    toggles: [bool; 6],
    selected: usize,
    editing: bool,
    error: Option<String>,
}

impl Form {
    fn new() -> Self {
        Self {
            values: [
                String::new(),
                String::new(),
                "csv".into(),
                "sqlite".into(),
                ",".into(),
                crate::config::DEFAULT_TABLE_NAME.into(),
                String::new(),
                crate::config::DEFAULT_BATCH_SIZE.to_string(),
            ],
            toggles: [false; 6],
            selected: 0,
            editing: false,
            error: None,
        }
    }

    fn value(&self, index: usize) -> String {
        match index {
            0..=7 => self.values[index].clone(),
            8..=13 => if self.toggles[index - 8] { "Yes" } else { "No" }.into(),
            _ => "Start conversion".into(),
        }
    }

    fn cycle(&mut self, direction: i8) {
        let mut options: Vec<&str> = match self.selected {
            2 => FORMATS.to_vec(),
            3 => TARGETS.to_vec(),
            _ => Vec::new(),
        };
        if self.selected == 3 && self.values[2] == "sqlite" {
            options.retain(|v| *v != "sqlite");
        }
        if !options.is_empty() {
            let current = options
                .iter()
                .position(|v| *v == self.values[self.selected])
                .unwrap_or(0);
            let next =
                (current as isize + direction as isize).rem_euclid(options.len() as isize) as usize;
            self.values[self.selected] = options[next].into();
        } else if (8..=13).contains(&self.selected) {
            self.toggles[self.selected - 8] = !self.toggles[self.selected - 8];
        }
    }

    fn command(&self) -> Result<Vec<String>> {
        if self.values[0].trim().is_empty() || self.values[1].trim().is_empty() {
            bail!("Input file and output name are required");
        }
        if self.toggles[0] && !matches!(self.values[2].as_str(), "csv" | "txt") {
            bail!("The doctor scan is available only for CSV and TXT input");
        }
        if self.values[2] == "sqlite" && self.values[3] == "sqlite" {
            bail!("SQLite-to-SQLite conversion is not supported");
        }
        let mut argv = vec![
            "DBMorph".into(),
            "-i".into(),
            self.values[0].clone(),
            "-o".into(),
            self.values[1].clone(),
            "-f".into(),
            self.values[2].clone(),
            "-t".into(),
            self.values[3].clone(),
            "-n".into(),
            self.values[5].clone(),
            "-b".into(),
            self.values[7].clone(),
        ];
        // Keep the default delimiter implicit so doctor detection can override it.
        if !self.values[4].is_empty() && self.values[4] != "," {
            argv.extend(["-d".into(), self.values[4].clone()]);
        }
        if !self.values[6].trim().is_empty() {
            argv.extend(["-e".into(), self.values[6].clone()]);
        }
        if self.toggles[1] {
            argv.extend(["--sqlite-mode".into(), "typed".into()]);
        }
        if self.toggles[2] {
            argv.push("--infer-types".into());
        }
        if self.toggles[3] {
            argv.push("--strict".into());
        }
        if self.toggles[4] {
            argv.push("--drop-existing".into());
        }
        if self.toggles[5] {
            argv.push("--no-header".into());
        }
        Ok(argv)
    }
}

enum Screen {
    Setup,
    Doctor(Vec<String>),
    Preview(Vec<Vec<String>>),
    Running,
    Complete(u64),
    Cancelled(bool),
    Failed(String),
}

struct DoctorChoice {
    args: AppArgs,
    report: doctor::Report,
}

pub async fn run() -> Result<u64> {
    ERROR_SHOWN.store(false, Ordering::Relaxed);
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("interactive mode requires a terminal; pass --help for CLI usage");
    }
    enable_raw_mode().context("could not enable terminal input mode")?;
    let mut out = stdout();
    if let Err(error) = execute!(out, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error.into());
    }
    let backend = CrosstermBackend::new(out);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(error) => {
            let _ = execute!(stdout(), LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error.into());
        }
    };
    let result = lifecycle(&mut terminal).await;
    drop(terminal);
    let _ = execute!(stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
    result
}

async fn lifecycle(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<u64> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    *sink()
        .lock()
        .map_err(|_| anyhow::anyhow!("TUI event lock poisoned"))? = Some(tx);
    let _sink_guard = ActiveSinkGuard;
    let mut form = Form::new();
    let mut screen = Screen::Setup;
    let mut doctor_choice: Option<DoctorChoice> = None;
    let mut job: Option<JoinHandle<Result<u64>>> = None;
    let mut pending_preview: Option<oneshot::Sender<bool>> = None;
    let mut logs = VecDeque::<String>::new();
    let mut progress = String::from("Waiting to start");

    let outcome = loop {
        terminal.draw(|frame| draw_lifecycle(frame, &form, &screen, &logs, &progress))?;

        if matches!(screen, Screen::Running) {
            if job.as_ref().is_some_and(JoinHandle::is_finished) {
                let task = job.take().unwrap();
                screen = match task.await {
                    Ok(Ok(rows)) => Screen::Complete(rows),
                    Ok(Err(error)) if error.to_string().contains("cancelled at preview") => {
                        Screen::Cancelled(true)
                    }
                    Ok(Err(error)) => Screen::Failed(format!("{error:#}")),
                    Err(error) => Screen::Failed(format!("conversion task failed: {error}")),
                };
                continue;
            }
            if let Ok(event) = rx.try_recv() {
                handle_event(
                    event,
                    &mut screen,
                    &mut pending_preview,
                    &mut logs,
                    &mut progress,
                );
                continue;
            }
            if event::poll(Duration::from_millis(40))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press
                        && matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
                    {
                        // Writers are not cooperatively cancellable once output starts.
                        push_log(
                            &mut logs,
                            "Conversion is running; cancel is disabled to avoid partial output"
                                .into(),
                        );
                    }
                }
            }
            continue;
        }

        if let Some(event) = rx.try_recv().ok() {
            handle_event(
                event,
                &mut screen,
                &mut pending_preview,
                &mut logs,
                &mut progress,
            );
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match &mut screen {
                Screen::Setup => handle_form_key(
                    key.code,
                    &mut form,
                    &mut screen,
                    &mut doctor_choice,
                    &mut job,
                )?,
                Screen::Doctor(_) => match key.code {
                    KeyCode::Char('a') => {
                        if let Some(mut choice) = doctor_choice.take() {
                            let applied = choice.report.apply_safe(&mut choice.args);
                            choice.report.print(&applied);
                            start_conversion(choice.args, &mut job, &mut screen);
                        }
                    }
                    KeyCode::Char('c') => {
                        if let Some(choice) = doctor_choice.take() {
                            choice.report.print(&[]);
                            start_conversion(choice.args, &mut job, &mut screen);
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => screen = Screen::Cancelled(false),
                    _ => {}
                },
                Screen::Preview(_) => match key.code {
                    KeyCode::Enter | KeyCode::Char('y') => {
                        if let Some(answer) = pending_preview.take() {
                            let _ = answer.send(true);
                        }
                        screen = Screen::Running;
                    }
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                        if let Some(answer) = pending_preview.take() {
                            let _ = answer.send(false);
                        }
                        screen = Screen::Running;
                    }
                    _ => {}
                },
                Screen::Running => unreachable!(),
                Screen::Complete(rows) => match key.code {
                    KeyCode::Enter | KeyCode::Char('q') => break Ok(*rows),
                    _ => {}
                },
                Screen::Cancelled(_) => match key.code {
                    KeyCode::Enter | KeyCode::Char('q') => {
                        break Err(anyhow::anyhow!("conversion cancelled by user"));
                    }
                    _ => {}
                },
                Screen::Failed(message) => match key.code {
                    KeyCode::Enter | KeyCode::Char('q') => {
                        break Err(anyhow::anyhow!(message.clone()));
                    }
                    _ => {}
                },
            }
        }
        if matches!(screen, Screen::Cancelled(_)) {
            terminal.draw(|frame| draw_lifecycle(frame, &form, &screen, &logs, &progress))?;
        }
    };
    if outcome.is_err() {
        ERROR_SHOWN.store(true, Ordering::Relaxed);
    }
    outcome
}

struct ActiveSinkGuard;

impl Drop for ActiveSinkGuard {
    fn drop(&mut self) {
        if let Ok(mut active_sink) = sink().lock() {
            *active_sink = None;
        }
    }
}

fn handle_form_key(
    key: KeyCode,
    form: &mut Form,
    screen: &mut Screen,
    doctor_choice: &mut Option<DoctorChoice>,
    job: &mut Option<JoinHandle<Result<u64>>>,
) -> Result<()> {
    if form.editing {
        match key {
            KeyCode::Enter | KeyCode::Esc => form.editing = false,
            KeyCode::Backspace => {
                form.values[form.selected].pop();
            }
            KeyCode::Char(c) => form.values[form.selected].push(c),
            _ => {}
        }
        return Ok(());
    }
    let key = if key == KeyCode::Char('s') {
        form.selected = LABELS.len() - 1;
        KeyCode::Enter
    } else {
        key
    };
    match key {
        KeyCode::Esc | KeyCode::Char('q') => *screen = Screen::Cancelled(false),
        KeyCode::Up | KeyCode::Char('k') => form.selected = form.selected.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => {
            form.selected = (form.selected + 1).min(LABELS.len() - 1)
        }
        KeyCode::Left => form.cycle(-1),
        KeyCode::Right => form.cycle(1),
        KeyCode::Char(' ') if (8..=13).contains(&form.selected) => {
            form.toggles[form.selected - 8] = !form.toggles[form.selected - 8]
        }
        KeyCode::Enter if (0..=1).contains(&form.selected) || (4..=7).contains(&form.selected) => {
            form.editing = true
        }
        KeyCode::Enter if form.selected == 2 || form.selected == 3 => form.cycle(1),
        KeyCode::Enter if (8..=13).contains(&form.selected) => {
            form.toggles[form.selected - 8] = !form.toggles[form.selected - 8]
        }
        KeyCode::Enter => {
            match form
                .command()
                .and_then(|argv| crate::args::parse_from(argv).map_err(anyhow::Error::from))
            {
                Ok(args) => {
                    if let Err(error) = validate_input(&args) {
                        form.error = Some(format!("{error:#}"));
                        return Ok(());
                    }
                    if let Err(error) = validate_pair(&args) {
                        form.error = Some(error.to_string());
                        return Ok(());
                    }
                    if doctor::applies(&args) && form.toggles[0] {
                        let report = match doctor::examine(&args) {
                            Ok(report) => report,
                            Err(error) => {
                                form.error = Some(format!("{error:#}"));
                                return Ok(());
                            }
                        };
                        let findings = report
                            .findings
                            .iter()
                            .map(|finding| match &finding.fix {
                                Some(fix) if finding.safe => format!(
                                    "{} [safe fix: {}]",
                                    finding.message,
                                    fix.flags().join(" ")
                                ),
                                Some(fix) => format!(
                                    "{} [suggested: {}]",
                                    finding.message,
                                    fix.flags().join(" ")
                                ),
                                None => finding.message.clone(),
                            })
                            .collect();
                        doctor_choice.replace(DoctorChoice { args, report });
                        *screen = Screen::Doctor(findings);
                    } else {
                        start_conversion(args, job, screen);
                    }
                }
                Err(error) => form.error = Some(format!("{error:#}")),
            }
        }
        _ => {}
    }
    if key != KeyCode::Enter {
        form.error = None;
    }
    Ok(())
}

fn start_conversion(args: AppArgs, job: &mut Option<JoinHandle<Result<u64>>>, screen: &mut Screen) {
    *job = Some(tokio::spawn(crate::app::run_pipeline(args)));
    *screen = Screen::Running;
}

fn validate_input(args: &AppArgs) -> Result<()> {
    if !std::path::Path::new(&args.input_path).is_file() {
        bail!("input path {:?} is not a regular file", args.input_path);
    }
    File::open(&args.input_path)
        .with_context(|| format!("cannot read input file {:?}", args.input_path))?;
    Ok(())
}

fn validate_pair(args: &AppArgs) -> Result<()> {
    if args.input_file_type == "sqlite" && args.database_type == "sqlite" {
        bail!("SQLite-to-SQLite conversion is not supported");
    }
    Ok(())
}

fn handle_event(
    event: UiEvent,
    screen: &mut Screen,
    pending: &mut Option<oneshot::Sender<bool>>,
    logs: &mut VecDeque<String>,
    progress: &mut String,
) {
    match event {
        UiEvent::Log(message) => push_log(logs, message),
        UiEvent::Progress(message) => *progress = message,
        UiEvent::Preview(rows, answer) => {
            *screen = Screen::Preview(rows);
            *pending = Some(answer);
        }
    }
}

fn push_log(logs: &mut VecDeque<String>, message: String) {
    if message.is_empty() {
        return;
    }
    logs.push_back(message);
    while logs.len() > 8 {
        logs.pop_front();
    }
}

fn draw_lifecycle(
    frame: &mut ratatui::Frame<'_>,
    form: &Form,
    screen: &Screen,
    logs: &VecDeque<String>,
    progress: &str,
) {
    match screen {
        Screen::Setup => draw_setup(frame, form),
        Screen::Doctor(findings) => {
            let mut lines: Vec<&str> = findings.iter().map(String::as_str).collect();
            lines.extend([
                "",
                "a  Apply safe fixes and continue",
                "c  Continue without fixes",
                "q  Cancel",
            ]);
            draw_message(frame, "CSV/TXT diagnostics", &lines);
        }
        Screen::Preview(rows) => draw_preview(frame, rows),
        Screen::Running => draw_running(frame, logs, progress),
        Screen::Complete(rows) if *rows == 0 => draw_message(
            frame,
            "Conversion complete",
            &[
                "No rows or values required repair or skipping.",
                "Enter / q  Close",
            ],
        ),
        Screen::Complete(rows) => {
            let details = format!("Rows repaired or skipped: {rows}");
            draw_message(
                frame,
                "Conversion complete",
                &[&details, "Enter / q  Close"],
            );
        }
        Screen::Cancelled(started) => draw_message(
            frame,
            "Conversion cancelled",
            &[
                if *started {
                    "Conversion stopped before the destination was opened."
                } else {
                    "No output was started."
                },
                "Enter / q  Close",
            ],
        ),
        Screen::Failed(error) => {
            draw_message(frame, "Conversion failed", &[error, "Enter / q  Close"])
        }
    }
}

fn draw_setup(frame: &mut ratatui::Frame<'_>, form: &Form) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());
    let title = form.error.as_deref().unwrap_or("DBMorph · data conversion");
    let style = if form.error.is_some() {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(title)
            .style(style)
            .block(Block::default().borders(Borders::ALL).title("Setup")),
        parts[0],
    );
    let items: Vec<ListItem> = LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let value = form.value(i);
            let edit = if form.editing && i == form.selected {
                " [editing]"
            } else {
                ""
            };
            ListItem::new(format!("{label:<22} {value}{edit}"))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(form.selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Options"))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        parts[1],
        &mut state,
    );
    frame.render_widget(
        Paragraph::new(if form.editing {
            "Type · Enter finish"
        } else {
            "↑/↓ select · ←/→ change · Enter edit/toggle · s start · q cancel"
        })
        .block(Block::default().borders(Borders::ALL).title("Keys")),
        parts[2],
    );
}

fn draw_message(frame: &mut ratatui::Frame<'_>, title: &str, lines: &[&str]) {
    let body = lines.join("\n");
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title(title)),
        frame.area(),
    );
}

fn draw_preview(frame: &mut ratatui::Frame<'_>, rows: &[Vec<String>]) {
    let area = frame.area();
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(3)])
        .split(area);
    let rendered: Vec<Row> = rows
        .iter()
        .take(10)
        .map(|r| Row::new(r.iter().take(8).cloned()))
        .collect();
    let widths = vec![
        Constraint::Ratio(1, rows.first().map_or(1, |r| r.len().max(1)) as u32);
        rows.first().map_or(1, |r| r.len().min(8).max(1))
    ];
    frame.render_widget(
        Table::new(rendered, widths).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Preview · confirm before writing"),
        ),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new("Enter / y  Continue     Esc / n  Cancel")
            .block(Block::default().borders(Borders::ALL)),
        parts[1],
    );
}

fn draw_running(frame: &mut ratatui::Frame<'_>, logs: &VecDeque<String>, progress: &str) {
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new("Converting…")
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL).title("DBMorph")),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new(logs.iter().cloned().collect::<Vec<_>>().join("\n"))
            .wrap(Wrap { trim: true })
            .block(Block::default().borders(Borders::ALL).title("Activity")),
        parts[1],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{progress} · cancellation is disabled while output is being written"
        ))
        .block(Block::default().borders(Borders::ALL).title("Progress")),
        parts[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(from: &str, to: &str) -> AppArgs {
        crate::args::parse_from([
            "DBMorph", "-i", "input", "-o", "output", "-f", from, "-t", to,
        ])
        .unwrap()
    }

    #[test]
    fn rejects_sqlite_to_sqlite_before_starting_a_writer() {
        let args = args("sqlite", "sqlite");
        assert!(
            validate_pair(&args)
                .unwrap_err()
                .to_string()
                .contains("not supported")
        );
    }

    #[test]
    fn accepts_a_supported_pair() {
        assert!(validate_pair(&args("csv", "sqlite")).is_ok());
    }

    #[test]
    fn input_must_be_readable_before_conversion() {
        let args = args("csv", "sqlite");
        assert!(validate_input(&args).is_err());
    }

    #[test]
    fn sqlite_target_is_removed_from_target_cycle_for_sqlite_source() {
        let mut form = Form::new();
        form.values[2] = "sqlite".into();
        form.selected = 3;
        form.cycle(1);
        assert_ne!(form.values[3], "sqlite");
    }
}
