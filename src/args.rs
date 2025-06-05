use clap::{value_parser, Arg, ArgAction, Command};

#[derive(Debug)]
pub struct AppArgs {
    pub input_path: String,
    pub output_path: String,
    pub table_name: String,
    pub column_count: i32,
    pub encoding: Option<String>,
    pub delimiter: Option<u8>,
    pub remove_rows: Option<usize>,
    pub custom_rows: Option<String>,
    pub database_type: String,
    pub database_url: Option<String>,
    pub database_user: Option<String>,
    pub database_pass: Option<String>,
    pub drop_existing: bool,
    pub headers_row: bool,
    pub batch_size: Option<u32>,
    pub input_file_type: Option<String>,
    pub threads: u32,
}

pub fn parse_args() -> AppArgs {
    // Defines and parses command-line arguments for the application using clap.
    // Configures options like input/output file paths, table name, encoding, delimiter,
    // database connection details, batch size, thread count, and flags for processing behavior.
    // Returns a structured object containing all parsed argument values.
    let matches = Command::new("DBMorph")
        .version("v0.3.4-dev")
        .author("DroBoV1tya")
        .about("A utility to parse data files and load them into an SQLite database.")
        .arg(
            Arg::new("input")
                .short('i')
                .long("input")
                .value_name("FILE_PATH")
                .help("Specifies the path to the input file")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("FILE_PATH")
                .help("Specifies the path for the output SQLite database file")
                .required(true)
                .num_args(1),
        )
        .arg(
            Arg::new("table_name")
                .short('T')
                .long("table-name")
                .value_name("TABLE_NAME")
                .help("Specifies the name of the table to create/use in the SQLite database")
                .required(false)
                .default_value("main")
                .num_args(1),
        )
        .arg(
            Arg::new("column_count")
                .short('c')
                .long("column-count")
                .value_name("COLUMN_COUNT")
                .help("Specifies the count of columns in the SQLite database")
                .required(false)
                .default_value("100")
                .num_args(1),
        )
        .arg(
            Arg::new("encoding")
                .short('e')
                .long("encoding")
                .value_name("ENCODING_NAME")
                .help("Specifies the file encoding. If not provided, encoding will be auto-detected if possible.")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("delimiter")
                .short('d')
                .long("delimiter")
                .value_name("DELIMITER_CHAR")
                .help("Specifies the field delimiter character for the input file (e.g., ',', 't'). Defaults to ',' if not provided.")
                .required(false)
                .default_value(",")
                .num_args(1),
        )
        .arg(
            Arg::new("remove_rows")
                .short('r')
                .long("remove-rows")
                .value_name("REMOVE_ROWS_COUNT")
                .help("Skips the specified number of initial rows in the input file before processing begins.")
                .required(false)
                .num_args(1)
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(
            Arg::new("custom_rows")
                .short('C')
                .long("custom-rows")
                .value_name("ID NAME ETC...")
                .help("Space-separated list of custom column names to override headers or auto-generated keys (e.g., name email phone)")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("database_type")
                .short('b')
                .long("database")
                .value_name("DATABASE_TYPE")
                .help("Specifies the database type. Default: sqlite")
                .required(false)
                .default_value("sqlite")
                .num_args(1),
        )
        .arg(
            Arg::new("database_url")
                .short('u')
                .long("db-url")
                .value_name("DATABASE_URL")
                .help("Specifies the database connection string.")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("database_user")
                .short('U')
                .long("db-user")
                .value_name("DATABASE_USER")
                .help("Specifies the database user.")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("database_pass")
                .short('p')
                .long("db-password")
                .value_name("DATABASE_PASSWORD")
                .help("Specifies the database password.")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("drop_existing")
                .short('x')
                .long("drop-existing")
                .help("If set, drops the target table if it already exists before inserting new data")
                .required(false)
                .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("headers_row")
                .short('H')
                .long("headers-row")
                .help("Use first row as headers for document keys instead of default column names (c1, c2, ...)")
                .required(false)
                .action(ArgAction::SetTrue)
        )
        .arg(
            Arg::new("batch_size")
                .short('s')
                .long("batch-size")
                .value_name("SIZE")
                .help("Specifies the number of rows to insert in a single batch/transaction. Defaults to a predefined value if not set. Default value: 10000")
                .required(false)
                .default_value("10000")
                .num_args(1)
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("input_file_type")
                .short('f')
                .long("input-filetype")
                .value_name("INPUT_FILE_TYPE")
                .help("Specifies the name of the table to create/use in the SQLite database")
                .required(false)
                .num_args(1),
        )
        .arg(
            Arg::new("threads")
                .short('t')
                .value_name("COUNT")
                .help("Specifies the number of worker threads to use for processing")
                .required(false)
                .default_value("1")
                .num_args(1)
                .value_parser(value_parser!(u32).range(1..)),
        )
        .get_matches();

    let input_path = matches
        .get_one::<String>("input")
        .expect("'input' argument is required and checked by clap.")
        .to_string();

    let output_path = matches
        .get_one::<String>("output")
        .expect("'output' argument is required and checked by clap.")
        .to_string();

    let table_name = matches
        .get_one::<String>("table_name")
        .expect("'table_name' argument is required and checked by clap.")
        .to_string();

    let encoding = matches.get_one::<String>("encoding").map(|s| s.to_string());

    let delimiter = matches
        .get_one::<String>("delimiter")
        .map(|s| match s.as_str() {
            "t" | "\\t" => b'\t',
            "n" | "\\n" => b'\n',
            "r" | "\\r" => b'\r',
            "," => b',',
            ";" => b';',
            "|" => b'|',
            _ => s.chars().next().unwrap_or(',') as u8,
        });

    let remove_rows = matches.get_one::<usize>("remove_rows").copied();

    let custom_rows: Option<String> = matches
        .get_one::<String>("custom_rows")
        .map(|r| r.to_string());

    let database_type: String = matches
        .get_one::<String>("database_type")
        .unwrap()
        .to_string();

    let database_url: Option<String> = matches
        .get_one::<String>("database_url")
        .map(|s| s.to_string());

    let database_user: Option<String> = matches
        .get_one::<String>("database_user")
        .map(|s| s.to_string());

    let database_pass: Option<String> = matches
        .get_one::<String>("database_pass")
        .map(|s| s.to_string());

    let column_count = matches
        .get_one::<String>("column_count")
        .expect("'column_count' argument is required and checked by clap.")
        .parse::<i32>()
        .unwrap();

    let drop_existing = matches.get_flag("drop_existing");

    let headers_row = matches.get_flag("headers_row");

    let batch_size = matches.get_one::<u32>("batch_size").copied();

    let input_file_type = matches
        .get_one::<String>("input_file_type")
        .map(|s| s.to_string());

    let threads = matches
        .get_one::<u32>("threads")
        .copied()
        .expect("'threads' argument has a default and is parsed by clap.");

    AppArgs {
        input_path,
        output_path,
        table_name,
        column_count,
        encoding,
        delimiter,
        remove_rows,
        custom_rows,
        database_type,
        database_url,
        database_user,
        database_pass,
        drop_existing,
        headers_row,
        batch_size,
        input_file_type,
        threads,
    }
}
