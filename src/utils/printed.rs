use tabled::{Table};

pub fn print_csv_table(records: &[Vec<String>]) {
    let mut preview: Vec<String> = Vec::new();

    for row in records.iter().take(5) {
        let row_str = row.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",");
        preview.push(row_str);
    }

    let table = Table::new(preview);
    println!("{}", table);
}

