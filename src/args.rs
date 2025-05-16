use clap::{Arg, ArgAction, Command, value_parser};

#[derive(Debug)]
pub struct AppArgs {
    pub input_path: String,
    pub output_path: String,
    pub table_name: String,
    pub column_count: i32,
    pub encoding: Option<String>,
    pub delimiter: Option<u8>,
    pub database_type: String,
    pub drop_existing: bool,
    pub batch_size: Option<u32>,
    pub threads: u32,
}

pub fn parse_args() -> AppArgs {
    let matches = Command::new("parseqlite")
        .version("0.0.2")
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
                .help("Specifies the field delimiter character for the input file (e.g., ',', '\\t'). Defaults to ',' if not provided.")
                .required(false)
                .default_value(",")
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
            Arg::new("drop_existing")
                .short('x')
                .long("drop-existing")
                .help("If set, drops the target table if it already exists before inserting new data")
                .required(false)
                .action(ArgAction::SetTrue),
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

    let database_type: String = matches
        .get_one::<String>("database_type")
        .unwrap()
        .to_string();

    let column_count = matches
        .get_one::<String>("column_count")
        .expect("'column_count' argument is required and checked by clap.")
        .parse::<i32>()
        .unwrap();

    let encoding = matches
        .get_one::<String>("encoding")
        .map(|s| s.to_string());

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

    let drop_existing = matches.get_flag("drop_existing");

    let batch_size = matches
        .get_one::<u32>("batch_size")
        .copied();

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
        database_type,
        drop_existing,
        batch_size,
        threads,
    }
}