# DBMorph

**DBMorph** is a high-performance Rust utility designed for fast and efficient processing of large text data files (CSV, TXT, SQL dumps, etc.) and loading them into databases, including SQLite.

The main goal is to provide a flexible, scalable ETL (Extract, Transform, Load) tool capable of handling very large files by minimizing memory usage via streaming and advanced parsing techniques.

---

## Key features

- **Speed and efficiency**  
  Rust ensures excellent performance and low resource consumption.

- **Handling large files**  
  Streaming parsing allows working with files significantly larger than available RAM.

- **Intelligent parsing**  
  - Automatic detection of input file encoding  
  - Heuristic detection of delimiters in CSV-like formats  
  - Support for variable column counts per row  
  - Parsing SQL dumps with interactive table selection and data export

- **Interactive CLI**  
  Enables selecting tables from SQL dumps for export and previewing the first 5 rows before exporting.

- **SQLite export**  
  Automatic creation of tables with FTS5 full-text search support and data export.

- **Scalability**  
  Architecture designed for future support of other popular DBMS (PostgreSQL, MySQL, ClickHouse) and formats.

---

## About the project

DBMorph aims to be a reliable and universal solution for developers and data analysts who need a fast, memory-efficient, and easy-to-use tool for moving and structuring large volumes of text data into databases for further analysis and usage.
