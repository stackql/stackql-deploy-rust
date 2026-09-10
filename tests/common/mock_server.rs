//! Minimal in-process StackQL (PostgreSQL wire protocol v3) mock server.
//!
//! Speaks just enough of the simple-query protocol for `PgwireLite`:
//! startup handshake, `Q` (Query), `X` (Terminate), and the response
//! messages `T`/`D`/`C`/`E`/`Z`. Every SQL statement received is recorded so
//! tests can assert on exactly what stackql-deploy sent.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// What the mock server answers for one SQL statement.
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// A result set. `None` cells are sent as SQL NULL.
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Option<String>>>,
    },
    /// A command tag such as `DELETE 1` or `INSERT 0 1`.
    Command(String),
    /// An `ErrorResponse` with the given message.
    Error(String),
}

impl MockResponse {
    /// Convenience: a result set with zero rows.
    pub fn empty() -> Self {
        MockResponse::Rows {
            columns: vec![],
            rows: vec![],
        }
    }

    /// Convenience: a single-row result set.
    pub fn single_row(cells: &[(&str, &str)]) -> Self {
        MockResponse::Rows {
            columns: cells.iter().map(|(c, _)| c.to_string()).collect(),
            rows: vec![cells.iter().map(|(_, v)| Some(v.to_string())).collect()],
        }
    }
}

type Handler = Box<dyn FnMut(&str) -> MockResponse + Send>;

/// A running mock server bound to an ephemeral loopback port.
pub struct MockServer {
    port: u16,
    queries: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Start a server whose responses are produced by `handler`.
    pub fn start<F>(handler: F) -> Self
    where
        F: FnMut(&str) -> MockResponse + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let port = listener.local_addr().unwrap().port();
        let queries: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread_queries = Arc::clone(&queries);
        let thread_stop = Arc::clone(&stop);
        let mut handler: Handler = Box::new(handler);

        let thread = thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    // Errors here mean the client hung up; that is normal at
                    // the end of a test.
                    let _ = serve_connection(stream, &mut handler, &thread_queries);
                }
            }
        });

        MockServer {
            port,
            queries,
            stop,
            thread: Some(thread),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Every SQL statement received so far, in order.
    pub fn queries(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }

    /// Statements containing `needle`.
    pub fn queries_containing(&self, needle: &str) -> Vec<String> {
        self.queries()
            .into_iter()
            .filter(|q| q.contains(needle))
            .collect()
    }

    /// True when at least one statement contains `needle`.
    pub fn received(&self, needle: &str) -> bool {
        !self.queries_containing(needle).is_empty()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so the thread can observe the stop flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Wire protocol
// ---------------------------------------------------------------------------

const SSL_REQUEST_CODE: i32 = 80877103;
const CANCEL_REQUEST_CODE: i32 = 80877102;

fn serve_connection(
    mut stream: TcpStream,
    handler: &mut Handler,
    log: &Arc<Mutex<Vec<String>>>,
) -> io::Result<()> {
    // Startup: length-prefixed messages without a type byte.
    loop {
        let len = read_i32(&mut stream)? as usize;
        let mut body = vec![0u8; len.saturating_sub(4)];
        stream.read_exact(&mut body)?;
        if body.len() < 4 {
            return Ok(());
        }
        let code = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
        if code == SSL_REQUEST_CODE {
            stream.write_all(b"N")?;
            continue;
        }
        if code == CANCEL_REQUEST_CODE {
            return Ok(());
        }
        break;
    }

    // AuthenticationOk, then ReadyForQuery (idle).
    write_msg(&mut stream, b'R', &0i32.to_be_bytes())?;
    write_msg(&mut stream, b'Z', b"I")?;

    loop {
        let mut msg_type = [0u8; 1];
        if stream.read_exact(&mut msg_type).is_err() {
            return Ok(()); // client closed the socket
        }
        let len = read_i32(&mut stream)? as usize;
        let mut body = vec![0u8; len.saturating_sub(4)];
        stream.read_exact(&mut body)?;

        match msg_type[0] {
            b'Q' => {
                let sql_bytes = body.strip_suffix(&[0u8]).unwrap_or(&body);
                let sql = String::from_utf8_lossy(sql_bytes).into_owned();
                log.lock().unwrap().push(sql.clone());
                let response = handler(&sql);
                write_response(&mut stream, response)?;
                write_msg(&mut stream, b'Z', b"I")?;
            }
            b'X' => return Ok(()),
            _ => {
                // Unsupported message: answer with an error so the client
                // does not hang, then return to idle.
                write_error(&mut stream, "mock server: unsupported message")?;
                write_msg(&mut stream, b'Z', b"I")?;
            }
        }
    }
}

fn write_response(stream: &mut TcpStream, response: MockResponse) -> io::Result<()> {
    match response {
        MockResponse::Rows { columns, rows } => {
            // RowDescription
            let mut t = Vec::new();
            t.extend_from_slice(&(columns.len() as i16).to_be_bytes());
            for col in &columns {
                t.extend_from_slice(col.as_bytes());
                t.push(0);
                t.extend_from_slice(&0i32.to_be_bytes()); // table OID
                t.extend_from_slice(&0i16.to_be_bytes()); // attribute number
                t.extend_from_slice(&25i32.to_be_bytes()); // type OID (text)
                t.extend_from_slice(&(-1i16).to_be_bytes()); // type size
                t.extend_from_slice(&(-1i32).to_be_bytes()); // type modifier
                t.extend_from_slice(&0i16.to_be_bytes()); // format (text)
            }
            write_msg(stream, b'T', &t)?;

            // DataRow per row
            for row in &rows {
                let mut d = Vec::new();
                d.extend_from_slice(&(row.len() as i16).to_be_bytes());
                for cell in row {
                    match cell {
                        Some(v) => {
                            d.extend_from_slice(&(v.len() as i32).to_be_bytes());
                            d.extend_from_slice(v.as_bytes());
                        }
                        None => d.extend_from_slice(&(-1i32).to_be_bytes()),
                    }
                }
                write_msg(stream, b'D', &d)?;
            }

            let tag = format!("SELECT {}\0", rows.len());
            write_msg(stream, b'C', tag.as_bytes())
        }
        MockResponse::Command(tag) => {
            let tag = format!("{}\0", tag);
            write_msg(stream, b'C', tag.as_bytes())
        }
        MockResponse::Error(msg) => write_error(stream, &msg),
    }
}

fn write_error(stream: &mut TcpStream, msg: &str) -> io::Result<()> {
    let mut e = Vec::new();
    e.push(b'S');
    e.extend_from_slice(b"ERROR\0");
    e.push(b'C');
    e.extend_from_slice(b"XX000\0");
    e.push(b'M');
    e.extend_from_slice(msg.as_bytes());
    e.push(0);
    e.push(0);
    write_msg(stream, b'E', &e)
}

fn write_msg(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> io::Result<()> {
    let mut msg = Vec::with_capacity(5 + payload.len());
    msg.push(msg_type);
    msg.extend_from_slice(&((payload.len() + 4) as i32).to_be_bytes());
    msg.extend_from_slice(payload);
    stream.write_all(&msg)
}

fn read_i32(stream: &mut TcpStream) -> io::Result<i32> {
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf)?;
    Ok(i32::from_be_bytes(buf))
}
