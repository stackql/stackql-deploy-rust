//! A tiny stateful stand-in for cloud providers behind the mock server.
//!
//! Each provider "table" (e.g. `databricks_account.provisioning.workspaces`)
//! holds at most one row and a present/absent flag. `SELECT` answers from
//! that row, `INSERT` marks the table present, `DELETE` marks it absent. This
//! is deliberately simple: `WHERE` clauses are ignored and the select list is
//! parsed only far enough to honour column names, `AS` aliases, `COUNT(*)`
//! and quoted / numeric literals.

use std::collections::HashMap;

use regex::Regex;

use super::mock_server::MockResponse;

#[derive(Debug, Clone, Default)]
struct TableState {
    present: bool,
    row: HashMap<String, String>,
}

/// Builder for a fake provider backend. Consume it with
/// [`FakeCloud::into_handler`] and pass the result to `MockServer::start`.
#[derive(Debug, Clone, Default)]
pub struct FakeCloud {
    providers: Vec<(String, String)>,
    tables: HashMap<String, TableState>,
    /// `(needle, message)`: any statement containing `needle` fails with
    /// `message` instead of being evaluated.
    errors: Vec<(String, String)>,
}

impl FakeCloud {
    /// `providers` uses the manifest form, e.g. `aws::v26.08.00444` or `aws`.
    /// Every listed provider is reported as installed by `SHOW PROVIDERS`.
    pub fn new(providers: &[&str]) -> Self {
        let providers = providers
            .iter()
            .map(|p| match p.split_once("::") {
                Some((name, version)) => (name.to_string(), version.to_string()),
                None => (p.to_string(), "v1.0.0".to_string()),
            })
            .collect();
        FakeCloud {
            providers,
            ..Default::default()
        }
    }

    /// The table exists and returns `row` for selects.
    pub fn table_present(mut self, table: &str, row: &[(&str, &str)]) -> Self {
        self.tables.insert(
            table.to_string(),
            TableState {
                present: true,
                row: to_row(row),
            },
        );
        self
    }

    /// The table does not exist. Selects return no rows; an `INSERT` makes it
    /// present with `row_after_create`.
    pub fn table_absent(mut self, table: &str, row_after_create: &[(&str, &str)]) -> Self {
        self.tables.insert(
            table.to_string(),
            TableState {
                present: false,
                row: to_row(row_after_create),
            },
        );
        self
    }

    /// Any statement containing `needle` fails with `message`.
    pub fn error_on(mut self, needle: &str, message: &str) -> Self {
        self.errors.push((needle.to_string(), message.to_string()));
        self
    }

    pub fn into_handler(mut self) -> impl FnMut(&str) -> MockResponse + Send + 'static {
        move |sql: &str| self.handle(sql)
    }

    fn handle(&mut self, sql: &str) -> MockResponse {
        for (needle, message) in &self.errors {
            if sql.contains(needle.as_str()) {
                return MockResponse::Error(message.clone());
            }
        }

        let trimmed = sql.trim().trim_end_matches(';').trim();
        let upper = trimmed.to_ascii_uppercase();

        if upper == "SHOW PROVIDERS" {
            return MockResponse::Rows {
                columns: vec!["name".to_string(), "version".to_string()],
                rows: self
                    .providers
                    .iter()
                    .map(|(n, v)| vec![Some(n.clone()), Some(v.clone())])
                    .collect(),
            };
        }
        if upper.starts_with("REGISTRY PULL") {
            return MockResponse::Command("REGISTRY".to_string());
        }
        if upper.starts_with("SELECT") {
            return self.handle_select(trimmed);
        }
        if let Some(table) = capture(r"(?is)^\s*INSERT\s+INTO\s+([\w.]+)", trimmed) {
            self.tables.entry(table).or_default().present = true;
            return MockResponse::Command("INSERT 0 1".to_string());
        }
        if let Some(table) = capture(r"(?is)^\s*DELETE\s+FROM\s+([\w.]+)", trimmed) {
            self.tables.entry(table).or_default().present = false;
            return MockResponse::Command("DELETE 1".to_string());
        }
        if capture(r"(?is)^\s*UPDATE\s+([\w.]+)", trimmed).is_some() {
            return MockResponse::Command("UPDATE 1".to_string());
        }
        MockResponse::empty()
    }

    fn handle_select(&self, sql: &str) -> MockResponse {
        let table = capture(r"(?is)\bFROM\s+([\w.]+)", sql);
        let select_list = select_list(sql);
        let items: Vec<(String, String)> = split_top_level_commas(&select_list)
            .into_iter()
            .map(|item| parse_select_item(&item))
            .collect();

        let state = table.as_deref().and_then(|t| self.tables.get(t));
        let present = match (&table, state) {
            (None, _) => true, // literal SELECT with no FROM
            (Some(_), Some(s)) => s.present,
            (Some(_), None) => false, // unknown table behaves as empty
        };

        let columns: Vec<String> = items.iter().map(|(_, alias)| alias.clone()).collect();

        let is_count = items
            .iter()
            .any(|(expr, _)| expr.to_ascii_uppercase().starts_with("COUNT("));
        if is_count {
            let cells = items
                .iter()
                .map(|(expr, _)| {
                    if expr.to_ascii_uppercase().starts_with("COUNT(") {
                        Some(if present { "1" } else { "0" }.to_string())
                    } else {
                        state.and_then(|s| s.row.get(&column_of(expr)).cloned())
                    }
                })
                .collect();
            return MockResponse::Rows {
                columns,
                rows: vec![cells],
            };
        }

        if !present {
            return MockResponse::Rows {
                columns,
                rows: vec![],
            };
        }

        let cells = items
            .iter()
            .map(|(expr, _)| {
                literal_value(expr)
                    .or_else(|| state.and_then(|s| s.row.get(&column_of(expr)).cloned()))
            })
            .collect();
        MockResponse::Rows {
            columns,
            rows: vec![cells],
        }
    }
}

fn to_row(cells: &[(&str, &str)]) -> HashMap<String, String> {
    cells
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn capture(pattern: &str, sql: &str) -> Option<String> {
    Regex::new(pattern)
        .unwrap()
        .captures(sql)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Text between `SELECT` and the top-level `FROM` (or end of statement).
fn select_list(sql: &str) -> String {
    let re = Regex::new(r"(?is)^\s*SELECT\s+(.*?)(?:\s+FROM\s+.*)?$").unwrap();
    re.captures(sql)
        .and_then(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
        .unwrap_or_default()
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '\'' => {
                in_quote = !in_quote;
                current.push(ch);
            }
            '(' if !in_quote => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_quote => {
                depth -= 1;
                current.push(ch);
            }
            ',' if !in_quote && depth == 0 => {
                items.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }
    items
}

/// `expr AS alias` -> (expr, alias); `expr` -> (expr, bare column name).
fn parse_select_item(item: &str) -> (String, String) {
    let re = Regex::new(r"(?is)^(.*?)\s+AS\s+(\w+)$").unwrap();
    if let Some(c) = re.captures(item) {
        return (
            c.get(1).unwrap().as_str().trim().to_string(),
            c.get(2).unwrap().as_str().to_string(),
        );
    }
    let expr = item.trim().to_string();
    let alias = column_of(&expr);
    (expr, alias)
}

/// Bare column name of an expression such as `t.col` or `col`.
fn column_of(expr: &str) -> String {
    expr.trim()
        .rsplit('.')
        .next()
        .unwrap_or(expr)
        .trim()
        .to_string()
}

/// `'text'` -> text, `123` -> 123, otherwise `None` (a column reference).
fn literal_value(expr: &str) -> Option<String> {
    let e = expr.trim();
    if e.len() >= 2 && e.starts_with('\'') && e.ends_with('\'') {
        return Some(e[1..e.len() - 1].to_string());
    }
    if e.parse::<f64>().is_ok() {
        return Some(e.to_string());
    }
    None
}
