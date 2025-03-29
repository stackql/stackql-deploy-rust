use postgres::Client;

pub struct QueryResultColumn {
    pub name: String,
}

pub struct QueryResultRow {
    pub values: Vec<String>,
}

pub enum QueryResult {
    Data {
        columns: Vec<QueryResultColumn>,
        rows: Vec<QueryResultRow>,
        #[allow(dead_code)]
        notices: Vec<String>,
    },
    Command(String),
    Empty,
}

pub fn execute_query(query: &str, client: &mut Client) -> Result<QueryResult, String> {
    match client.simple_query(query) {
        Ok(results) => {
            let mut columns = Vec::new();
            let mut rows = Vec::new();
            let mut command_message = String::new();

            for result in results {
                match result {
                    postgres::SimpleQueryMessage::Row(row) => {
                        if columns.is_empty() {
                            for i in 0..row.len() {
                                columns.push(QueryResultColumn {
                                    name: row.columns()[i].name().to_string(),
                                });
                            }
                        }

                        let row_values = (0..row.len())
                            .map(|i| row.get(i).unwrap_or("NULL").to_string())
                            .collect();

                        rows.push(QueryResultRow { values: row_values });
                    }
                    postgres::SimpleQueryMessage::CommandComplete(cmd) => {
                        command_message = cmd.to_string();
                    }
                    _ => {}
                }
            }

            if !columns.is_empty() {
                Ok(QueryResult::Data {
                    columns,
                    rows,
                    notices: vec![],
                })
            } else if !command_message.is_empty() {
                Ok(QueryResult::Command(command_message))
            } else {
                Ok(QueryResult::Empty)
            }
        }
        Err(e) => Err(format!("Query execution failed: {}", e)),
    }
}
