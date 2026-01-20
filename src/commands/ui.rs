//! Web UI command for TideORM CLI
//!
//! Provides a browser-based interface for TideORM operations
//! with a query playground and model generator.

use colored::Colorize;
use std::io::Cursor;
use std::path::Path;
use tiny_http::{Header, Method, Response, Server};

/// Embedded static files from src/ui/
const HTML_CONTENT: &str = include_str!("../ui/index.html");
const CSS_CONTENT: &str = include_str!("../ui/style.css");
const JS_CONTENT: &str = include_str!("../ui/app.js");

/// Run the TideORM Studio web UI server
pub async fn run(host: &str, port: u16, verbose: bool) -> Result<(), String> {
    let addr = format!("{}:{}", host, port);
    
    println!("{}", "━".repeat(60).cyan());
    println!("{}", "🌊 TideORM Studio".bright_cyan().bold());
    println!("{}", "━".repeat(60).cyan());
    println!();
    println!("  {} http://{}", "Starting server at:".bright_white(), addr.bright_green());
    println!();
    println!("  {} Open the URL above in your browser", "→".cyan());
    println!("  {} Press {} to stop the server", "→".cyan(), "Ctrl+C".yellow());
    println!();
    println!("{}", "━".repeat(60).cyan());
    
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("Failed to start server: {}", e));
        }
    };
    
    // Check if tideorm.toml exists in current directory
    let config_exists = Path::new("tideorm.toml").exists();
    
    if !config_exists {
        println!();
        println!("  {} No tideorm.toml found in current directory", "⚠ Warning:".yellow().bold());
        println!("  {} Some CLI features will be disabled", "→".yellow());
        println!("  {} Run 'tideorm init' to create a project configuration", "→".yellow());
        println!();
    }
    
    if verbose {
        println!("{} Listening for requests...", "Info:".cyan());
    }
    
    // Main request loop
    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();
        
        if verbose {
            println!("  {} {} {}", "←".dimmed(), method.to_string().cyan(), url.dimmed());
        }
        
        let response = match (method, url.as_str()) {
            // Serve static files
            (Method::Get, "/" | "/index.html") => {
                create_response(HTML_CONTENT, "text/html; charset=utf-8")
            }
            (Method::Get, "/style.css") => {
                create_response(CSS_CONTENT, "text/css; charset=utf-8")
            }
            (Method::Get, "/app.js") => {
                create_response(JS_CONTENT, "application/javascript; charset=utf-8")
            }
            
            // API endpoints
            (Method::Get, "/api/config-check") => {
                let config_exists = Path::new("tideorm.toml").exists();
                let json = format!(r#"{{"exists": {}}}"#, config_exists);
                create_response(&json, "application/json")
            }
            
            (Method::Post, "/api/execute") => {
                handle_execute_request(&mut request, verbose)
            }
            
            (Method::Post, "/api/query") => {
                handle_query_request(&mut request, verbose)
            }
            
            // 404 for everything else
            _ => {
                create_response(r#"{"error": "Not found"}"#, "application/json")
            }
        };
        
        if let Err(e) = request.respond(response) {
            if verbose {
                eprintln!("  {} Failed to send response: {}", "Error:".red(), e);
            }
        }
    }
    
    Ok(())
}

/// Create an HTTP response with the given content and content type
fn create_response(content: &str, content_type: &str) -> Response<Cursor<Vec<u8>>> {
    let data = content.as_bytes().to_vec();
    let len = data.len();
    
    Response::from_data(data)
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap()
        )
        .with_header(
            Header::from_bytes(&b"Content-Length"[..], len.to_string().as_bytes()).unwrap()
        )
        .with_header(
            Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap()
        )
}

/// Handle CLI command execution requests
fn handle_execute_request(
    request: &mut tiny_http::Request,
    verbose: bool
) -> Response<Cursor<Vec<u8>>> {
    // Check if tideorm.toml exists
    if !Path::new("tideorm.toml").exists() {
        return create_response(
            r#"{"success": false, "error": "No tideorm.toml found. Run 'tideorm init' first."}"#,
            "application/json"
        );
    }
    
    // Read the request body
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut request.as_reader(), &mut body) {
        return create_response(
            &format!(r#"{{"success": false, "error": "Failed to read request: {}"}}"#, e),
            "application/json"
        );
    }
    
    // Parse the command from JSON
    let command = extract_json_field(&body, "command");
    
    if command.is_empty() {
        return create_response(
            r#"{"success": false, "error": "No command provided"}"#,
            "application/json"
        );
    }
    
    if verbose {
        println!("  {} Executing: tideorm {}", "→".cyan(), command.yellow());
    }
    
    // Execute the tideorm command
    let output = std::process::Command::new("tideorm")
        .args(command.split_whitespace())
        .output();
    
    match output {
        Ok(result) => {
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);
            let success = result.status.success();
            
            let output_text = if stdout.is_empty() {
                stderr.to_string()
            } else {
                format!("{}{}", stdout, if stderr.is_empty() { "".to_string() } else { format!("\n{}", stderr) })
            };
            
            // Escape the output for JSON
            let escaped = escape_json_string(&output_text);
            
            create_response(
                &format!(r#"{{"success": {}, "output": "{}"}}"#, success, escaped),
                "application/json"
            )
        }
        Err(e) => {
            create_response(
                &format!(r#"{{"success": false, "error": "Failed to execute command: {}"}}"#, e),
                "application/json"
            )
        }
    }
}

/// Handle SQL query execution requests
fn handle_query_request(
    request: &mut tiny_http::Request,
    verbose: bool
) -> Response<Cursor<Vec<u8>>> {
    // Read the request body
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut request.as_reader(), &mut body) {
        return create_response(
            &format!(r#"{{"success": false, "error": "Failed to read request: {}"}}"#, e),
            "application/json"
        );
    }
    
    // Parse the query from JSON
    let query = extract_json_field(&body, "query");
    
    if query.is_empty() {
        return create_response(
            r#"{"success": false, "error": "No query provided"}"#,
            "application/json"
        );
    }
    
    if verbose {
        println!("  {} Executing query: {}", "→".cyan(), query.yellow());
    }
    
    // For now, we'll simulate query execution
    // In a full implementation, this would connect to the actual database
    let result = format!(
        "Query received: {}\n\nNote: Direct SQL execution requires database connection configuration.\nUse 'tideorm db' commands for database operations.",
        query
    );
    
    let escaped = escape_json_string(&result);
    
    create_response(
        &format!(r#"{{"success": true, "result": "{}"}}"#, escaped),
        "application/json"
    )
}

/// Simple JSON field extractor (avoids adding serde dependency just for this)
fn extract_json_field(json: &str, field: &str) -> String {
    let pattern = format!(r#""{}""#, field);
    if let Some(start) = json.find(&pattern) {
        let rest = &json[start + pattern.len()..];
        // Skip : and whitespace
        let rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
        
        if rest.starts_with('"') {
            // String value
            let rest = &rest[1..];
            if let Some(end) = rest.find('"') {
                return rest[..end].to_string();
            }
        }
    }
    String::new()
}

/// Escape a string for JSON embedding
fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
