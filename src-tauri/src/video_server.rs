use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use tiny_http::{Server, Response, Request, Header, StatusCode};

pub struct VideoServer {
    port: u16,
    allowed_files: Arc<Mutex<HashMap<String, PathBuf>>>, // URL path -> file path
}

impl VideoServer {
    pub fn new() -> Result<Self, String> {
        // Bind to localhost on a random available port
        let server = Server::http("127.0.0.1:0")
            .map_err(|e| format!("Failed to create HTTP server: {}", e))?;

        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            _ => return Err("Unexpected server address type".to_string()),
        };
        let allowed_files = Arc::new(Mutex::new(HashMap::new()));

        eprintln!("[Video Server] Started on port {}", port);

        // Spawn server thread
        let files = allowed_files.clone();
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                handle_request(request, &files);
            }
        });

        Ok(Self {
            port,
            allowed_files,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn add_file(&self, url_path: String, file_path: PathBuf) {
        eprintln!("[Video Server] Adding file: {} -> {:?}", url_path, file_path);
        let mut files = self.allowed_files.lock().unwrap();
        files.insert(url_path, file_path);
    }

    pub fn get_url(&self, url_path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, url_path)
    }
}

fn handle_request(request: Request, allowed_files: &Arc<Mutex<HashMap<String, PathBuf>>>) {
    let url = request.url();
    let method = request.method();

    eprintln!("[Video Server] {} {}", method, url);

    // Get file path
    let files = allowed_files.lock().unwrap();
    let file_path = match files.get(url) {
        Some(path) => path.clone(),
        None => {
            eprintln!("[Video Server] File not found: {}", url);
            let response = Response::from_string("Not Found")
                .with_status_code(StatusCode(404));
            let _ = request.respond(response);
            return;
        }
    };
    drop(files);

    // Open file
    let mut file = match File::open(&file_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[Video Server] Failed to open file: {}", e);
            let response = Response::from_string("Internal Server Error")
                .with_status_code(StatusCode(500));
            let _ = request.respond(response);
            return;
        }
    };

    // Get file size
    let file_size = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(e) => {
            eprintln!("[Video Server] Failed to get file metadata: {}", e);
            let response = Response::from_string("Internal Server Error")
                .with_status_code(StatusCode(500));
            let _ = request.respond(response);
            return;
        }
    };

    // Check for Range header - convert to owned String to avoid borrow issues
    let range_header = request.headers().iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());

    match range_header {
        Some(range) if range.starts_with("bytes=") => {
            // Parse range request
            eprintln!("[Video Server] Range request: {}", range);
            handle_range_request(request, file, file_size, &range, &file_path);
        }
        _ => {
            // Serve entire file
            eprintln!("[Video Server] Full file request");
            handle_full_request(request, file, file_size, &file_path);
        }
    }
}

fn handle_range_request(
    request: Request,
    mut file: File,
    file_size: u64,
    range: &str,
    file_path: &PathBuf,
) {
    // Parse "bytes=start-end"
    let range = range.trim_start_matches("bytes=");
    let parts: Vec<&str> = range.split('-').collect();

    let start = parts[0].parse::<u64>().unwrap_or(0);
    let end = if parts.len() > 1 && !parts[1].is_empty() {
        parts[1].parse::<u64>().unwrap_or(file_size - 1)
    } else {
        file_size - 1
    };

    let length = end - start + 1;

    // Seek to start position
    if let Err(e) = file.seek(SeekFrom::Start(start)) {
        eprintln!("[Video Server] Failed to seek: {}", e);
        let response = Response::from_string("Internal Server Error")
            .with_status_code(StatusCode(500));
        let _ = request.respond(response);
        return;
    }

    // Read the requested range
    let mut buffer = vec![0u8; length as usize];
    if let Err(e) = file.read_exact(&mut buffer) {
        eprintln!("[Video Server] Failed to read range: {}", e);
        let response = Response::from_string("Internal Server Error")
            .with_status_code(StatusCode(500));
        let _ = request.respond(response);
        return;
    }

    // Determine content type
    let content_type = get_content_type(file_path);

    // Send 206 Partial Content response
    let content_range = format!("bytes {}-{}/{}", start, end, file_size);
    let response = Response::from_data(buffer)
        .with_status_code(StatusCode(206))
        .with_header(Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap())
        .with_header(Header::from_bytes(&b"Content-Length"[..], length.to_string().as_bytes()).unwrap())
        .with_header(Header::from_bytes(&b"Content-Range"[..], content_range.as_bytes()).unwrap())
        .with_header(Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());

    let _ = request.respond(response);
}

fn handle_full_request(
    request: Request,
    file: File,
    file_size: u64,
    file_path: &PathBuf,
) {
    // Determine content type
    let content_type = get_content_type(file_path);

    // Send 200 OK response using from_file to avoid chunked encoding
    let response = Response::from_file(file)
        .with_status_code(StatusCode(200))
        .with_chunked_threshold(usize::MAX)  // Force Content-Length instead of chunked
        .with_header(Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap())
        .with_header(Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap());

    let _ = request.respond(response);
}

fn get_content_type(path: &PathBuf) -> String {
    match path.extension().and_then(|s| s.to_str()) {
        Some("webm") => "video/webm".to_string(),
        Some("mp4") => "video/mp4".to_string(),
        Some("mkv") => "video/x-matroska".to_string(),
        Some("avi") => "video/x-msvideo".to_string(),
        Some("mov") => "video/quicktime".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}
