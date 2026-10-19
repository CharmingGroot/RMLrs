// SPDX-License-Identifier: Apache-2.0
//! CSV sources (RML-IO-Registry, `rml:CSV`): RFC 4180 with a header row.
//!
//! One iteration is one row; a reference is a column name. A column that does
//! not exist gives NULL (no value), while an empty field is the empty string.
//! CSV has no data types, so every value is an `xsd:string`.
//!
//! A CSVW table description (`csvw:Table`) can change how the file is split
//! into rows and cells: see [`CsvDialect`]. Without one, the file is plain
//! RFC 4180 with one header row.
//!
//! Rows are read one at a time, so memory does not grow with the file.

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::Arc;

use rmlrs_model::{CsvDialect, Error, Result, Trim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    columns: Arc<HashMap<String, usize>>,
    values: Vec<String>,
}

impl Row {
    pub fn get(&self, column: &str) -> Option<&str> {
        self.columns
            .get(column)
            .and_then(|&i| self.values.get(i))
            .map(String::as_str)
    }
}

/// Reads rows lazily from a buffered reader.
pub struct Rows<R> {
    reader: R,
    dialect: CsvDialect,
    /// Column names; `None` until the first row when there is no header.
    columns: Option<Arc<HashMap<String, usize>>>,
    line: String,
    /// The byte order mark, if any, has been removed.
    started: bool,
    done: bool,
}

impl<R> std::fmt::Debug for Rows<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rows")
            .field("columns", &self.columns.as_ref().map(|c| c.len()))
            .finish()
    }
}

impl<R: BufRead> Rows<R> {
    /// Reads the header row of plain RFC 4180 CSV (`rml:CSV`).
    pub fn new(reader: R) -> Result<Self> {
        Self::with_dialect(reader, CsvDialect::default())
    }

    /// Skips the leading rows and reads the header rows of a CSV file read
    /// with `dialect` (a CSVW table description). The first header row
    /// names the columns.
    pub fn with_dialect(reader: R, dialect: CsvDialect) -> Result<Self> {
        let mut rows = Rows {
            reader,
            dialect,
            columns: None,
            line: String::new(),
            started: false,
            done: false,
        };
        // Skipped rows are counted whatever they hold, comments included.
        for _ in 0..rows.dialect.skip_rows {
            rows.line.clear();
            if rows.read_line()? == 0 {
                break;
            }
            rows.finish_record()?;
        }
        let mut names = None;
        for _ in 0..rows.dialect.header_rows {
            match rows.next_record()? {
                Some(header) => {
                    names.get_or_insert(header);
                }
                None => break,
            }
        }
        if rows.dialect.header_rows > 0 {
            let columns = names
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(i, name)| (name, i))
                .collect();
            rows.columns = Some(Arc::new(columns));
        }
        Ok(rows)
    }

    /// Reads more lines into `line` while a quoted field is open.
    fn finish_record(&mut self) -> Result<()> {
        while !record_complete(&self.line, &self.dialect) {
            if self.read_line()? == 0 {
                return Err(Error::Source("CSV: unterminated quoted field".into()));
            }
        }
        Ok(())
    }

    /// The cells of the next record that is not a comment. A record may
    /// span several lines when a quoted field contains line breaks. `None`
    /// at end of input.
    fn next_record(&mut self) -> Result<Option<Vec<String>>> {
        loop {
            self.line.clear();
            if self.read_line()? == 0 {
                return Ok(None);
            }
            // A comment is one line, whatever quotes it holds.
            if let Some(prefix) = &self.dialect.comment_prefix
                && self.line.starts_with(prefix.as_str())
            {
                continue;
            }
            self.finish_record()?;
            let mut cells = parse_record(&self.line, &self.dialect);
            cells.drain(..self.dialect.skip_columns.min(cells.len()));
            for cell in &mut cells {
                trim(cell, self.dialect.trim);
            }
            return Ok(Some(cells));
        }
    }

    fn read_line(&mut self) -> Result<usize> {
        let n = self
            .reader
            .read_line(&mut self.line)
            .map_err(|e| Error::Source(format!("CSV: {e}")))?;
        if !self.started && n > 0 {
            self.started = true;
            if self.line.starts_with(BOM) {
                self.line.drain(..BOM.len_utf8());
            }
        }
        Ok(n)
    }

    /// The column names read from the header row, in order (none without
    /// a header row).
    pub fn header(&self) -> Vec<String> {
        let Some(columns) = &self.columns else {
            return Vec::new();
        };
        let mut names: Vec<(&String, &usize)> = columns.iter().collect();
        names.sort_by_key(|(_, i)| **i);
        names.into_iter().map(|(n, _)| n.clone()).collect()
    }
}

const BOM: char = '\u{feff}';

impl<R: BufRead> Iterator for Rows<R> {
    type Item = Result<Row>;

    fn next(&mut self) -> Option<Result<Row>> {
        if self.done {
            return None;
        }
        loop {
            match self.next_record() {
                Ok(None) => {
                    self.done = true;
                    return None;
                }
                Ok(Some(values)) if values.len() == 1 && values[0].is_empty() => continue,
                Ok(Some(values)) => {
                    // Without a header, columns get the names CSVW gives them.
                    let columns = self.columns.get_or_insert_with(|| {
                        Arc::new(
                            (0..values.len())
                                .map(|i| (format!("_col.{}", i + 1), i))
                                .collect(),
                        )
                    });
                    let width = columns.len();
                    if values.len() != width {
                        self.done = true;
                        return Some(Err(Error::Source(format!(
                            "CSV row has {} fields, header has {width}",
                            values.len()
                        ))));
                    }
                    return Some(Ok(Row {
                        columns: columns.clone(),
                        values,
                    }));
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

/// Parses a whole CSV document (convenience for tests and small inputs).
pub fn rows(text: &str) -> Result<Vec<Row>> {
    Rows::new(text.as_bytes())?.collect()
}

/// Parses a whole CSV document read with `dialect`.
pub fn rows_with_dialect(text: &str, dialect: CsvDialect) -> Result<Vec<Row>> {
    Rows::with_dialect(text.as_bytes(), dialect)?.collect()
}

fn trim(cell: &mut String, trim: Trim) {
    if matches!(trim, Trim::End | Trim::Both) {
        cell.truncate(cell.trim_end().len());
    }
    if matches!(trim, Trim::Start | Trim::Both) {
        let start = cell.len() - cell.trim_start().len();
        cell.drain(..start);
    }
}

/// Removes the line break that ends a record.
fn without_line_break(text: &str) -> &str {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.strip_suffix('\r').unwrap_or(text)
}

/// False while a quoted field is still open at the end of `text`.
fn record_complete(text: &str, dialect: &CsvDialect) -> bool {
    scan(without_line_break(text), dialect, |_| {})
}

/// Splits one complete record into cells.
fn parse_record(text: &str, dialect: &CsvDialect) -> Vec<String> {
    let mut row = Vec::new();
    let mut field = String::new();
    scan(without_line_break(text), dialect, |event| match event {
        Event::Char(c) => field.push(c),
        Event::EndOfField => row.push(std::mem::take(&mut field)),
    });
    row.push(field);
    row
}

enum Event {
    Char(char),
    EndOfField,
}

/// Walks a record, reporting cell characters and cell ends. Returns false
/// if a quoted field is still open at the end.
fn scan(text: &str, dialect: &CsvDialect, mut emit: impl FnMut(Event)) -> bool {
    let quote = dialect.quote;
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    let mut at_field_start = true;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == quote && dialect.double_quote && chars.peek() == Some(&quote) {
                emit(Event::Char(quote));
                chars.next();
            } else if c == '\\' && !dialect.double_quote && chars.peek().is_some() {
                emit(Event::Char(chars.next().expect("peeked")));
            } else if c == quote {
                in_quotes = false;
            } else {
                emit(Event::Char(c));
            }
            continue;
        }
        if c == quote && at_field_start {
            in_quotes = true;
            at_field_start = false;
        } else if c == dialect.delimiter {
            emit(Event::EndOfField);
            at_field_start = true;
        } else {
            emit(Event::Char(c));
            at_field_start = false;
        }
    }
    !in_quotes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_quotes_and_missing_columns() {
        let r = rows("ID,Name,Note\r\n10,\"Venus, W\",\n20,\"say \"\"hi\"\"\",x\n").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].get("Name"), Some("Venus, W"));
        assert_eq!(r[0].get("Note"), Some(""));
        assert_eq!(r[1].get("Name"), Some("say \"hi\""));
        assert_eq!(r[1].get("Missing"), None);
    }

    #[test]
    fn quoted_line_breaks_and_bom() {
        let r = rows("\u{feff}a,b\n\"x\ny\",2\n\n3,4").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].get("a"), Some("x\ny"));
        assert_eq!(r[1].get("b"), Some("4"));
    }

    #[test]
    fn ragged_rows_are_errors() {
        assert!(rows("a,b\n1\n").is_err());
        assert!(rows("a\n\"open\n").is_err());
    }

    fn dialect(f: impl FnOnce(&mut CsvDialect)) -> CsvDialect {
        let mut d = CsvDialect::default();
        f(&mut d);
        d
    }

    #[test]
    fn plain_csv_keeps_every_cell_as_written() {
        let r = rows("ID,Name\n#1, Venus \n").unwrap();
        assert_eq!(r[0].get("ID"), Some("#1"));
        assert_eq!(r[0].get("Name"), Some(" Venus "));
    }

    #[test]
    fn delimiter() {
        let d = dialect(|d| d.delimiter = ';');
        let r = rows_with_dialect("ID;Name\n10;Venus, W\n", d).unwrap();
        assert_eq!(r[0].get("Name"), Some("Venus, W"));
        let d = dialect(|d| d.delimiter = '\t');
        let r = rows_with_dialect("ID\tName\n10\tVenus\n", d).unwrap();
        assert_eq!(r[0].get("Name"), Some("Venus"));
    }

    #[test]
    fn quote_char() {
        let d = dialect(|d| d.quote = '\'');
        let r = rows_with_dialect("'id','name'\n0,'Geller, M'\n1,\"x\"\n", d).unwrap();
        assert_eq!(r[0].get("id"), Some("0"));
        assert_eq!(r[0].get("name"), Some("Geller, M"));
        assert_eq!(r[1].get("name"), Some("\"x\""));
    }

    #[test]
    fn double_quote_off_means_backslash_escapes() {
        let d = dialect(|d| d.double_quote = false);
        let r = rows_with_dialect("a,b\n\"say \\\"hi\\\"\",\"c:\\\\x\"\n", d).unwrap();
        assert_eq!(r[0].get("a"), Some("say \"hi\""));
        assert_eq!(r[0].get("b"), Some("c:\\x"));
        // With doubled quotes (the default), a backslash is an ordinary character.
        let r = rows("a\n\"c:\\x\"\n").unwrap();
        assert_eq!(r[0].get("a"), Some("c:\\x"));
    }

    #[test]
    fn trim_whitespace() {
        let text = "ID,Name\n  10  , Venus\n";
        let get = |t| rows_with_dialect(text, dialect(|d| d.trim = t)).unwrap()[0].clone();
        assert_eq!(get(Trim::Both).get("ID"), Some("10"));
        assert_eq!(get(Trim::Start).get("ID"), Some("10  "));
        assert_eq!(get(Trim::End).get("ID"), Some("  10"));
        assert_eq!(get(Trim::None).get("ID"), Some("  10  "));
        // Header cells are trimmed too.
        let r = rows_with_dialect(" ID ,Name\n1,x\n", dialect(|d| d.trim = Trim::Both)).unwrap();
        assert_eq!(r[0].get("ID"), Some("1"));
    }

    #[test]
    fn comment_prefix() {
        let d = dialect(|d| d.comment_prefix = Some("#".into()));
        let r = rows_with_dialect("# comments \"\nID,Name\n10,Venus\n# comment\n", d).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].get("ID"), Some("10"));
    }

    #[test]
    fn skip_rows_and_header_row_count() {
        let d = dialect(|d| {
            d.skip_rows = 2;
            d.header_rows = 2;
        });
        let r = rows_with_dialect("junk\nmore,junk\nID,Name\nid,name\n10,Venus\n", d).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].get("ID"), Some("10"));
        assert_eq!(r[0].get("id"), None);
        // A skipped row is one record, even across lines.
        let d = dialect(|d| d.skip_rows = 1);
        let r = rows_with_dialect("\"a\nb\"\nID\n10\n", d).unwrap();
        assert_eq!(r[0].get("ID"), Some("10"));
    }

    #[test]
    fn no_header_names_columns_by_number() {
        let d = dialect(|d| d.header_rows = 0);
        let r = rows_with_dialect("10,Venus\n11,Serena\n", d).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].get("_col.1"), Some("10"));
        assert_eq!(r[1].get("_col.2"), Some("Serena"));
    }

    #[test]
    fn skip_columns() {
        let d = dialect(|d| d.skip_columns = 1);
        let r = rows_with_dialect("x,ID,Name\n?,10,Venus\n", d).unwrap();
        assert_eq!(r[0].get("ID"), Some("10"));
        assert_eq!(r[0].get("x"), None);
    }

    #[test]
    fn bom_before_skipped_rows() {
        let d = dialect(|d| d.skip_rows = 1);
        let r = rows_with_dialect("\u{feff}junk\nID\n10\n", d).unwrap();
        assert_eq!(r[0].get("ID"), Some("10"));
    }
}
