use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

pub fn serve(files: Vec<(String, Vec<u8>)>) -> String {
    serve_with(|_| files)
}

pub fn serve_with(build: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let files = build(&base);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let files = files.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut request = String::new();
                    if reader.read_line(&mut request).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let mut range: Option<(usize, Option<usize>)> = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let line = line.trim().to_ascii_lowercase();
                        if line.is_empty() {
                            break;
                        }
                        if let Some(value) = line.strip_prefix("range: bytes=") {
                            let mut parts = value.split('-');
                            let start =
                                parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
                            let end = parts.next().and_then(|part| part.parse().ok());
                            range = Some((start, end));
                        }
                    }
                    let Some((_, body)) = files.iter().find(|(name, _)| *name == path) else {
                        let _ = stream
                            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
                        return;
                    };
                    let reply = match range {
                        Some((start, end)) => {
                            let end = end.unwrap_or(body.len() - 1).min(body.len() - 1);
                            let slice = &body[start..=end];
                            let head = format!(
                                "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {start}-{end}/{}\r\ncontent-length: {}\r\n\r\n",
                                body.len(),
                                slice.len()
                            );
                            [head.into_bytes(), slice.to_vec()].concat()
                        }
                        None => {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n",
                                body.len()
                            );
                            [head.into_bytes(), body.clone()].concat()
                        }
                    };
                    if stream.write_all(&reply).is_err() {
                        return;
                    }
                    let _ = reader.by_ref().take(0).read(&mut []);
                }
            });
        }
    });
    base
}
