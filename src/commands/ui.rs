//! Web UI command for TideORM CLI
//!
//! Provides a browser-based interface for TideORM operations
//! with a query playground and model generator.

use colored::Colorize;
use crate::{config::TideConfig, runtime_db};
use serde::Deserialize;
use serde_json::json;
use std::io::Cursor;
use std::path::Path;
use tiny_http::{Header, Method, Response, Server};

#[derive(Deserialize)]
struct ExecuteRequest {
    command: Vec<String>,
}

#[derive(Deserialize)]
struct QueryRequest {
    query: String,
}

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
        
        if let Err(e) = request.respond(response)
            && verbose {
            eprintln!("  {} Failed to send response: {}", "Error:".red(), e);
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

fn create_json_response(value: serde_json::Value) -> Response<Cursor<Vec<u8>>> {
    create_response(&value.to_string(), "application/json")
}

/// Handle CLI command execution requests
fn handle_execute_request(
    request: &mut tiny_http::Request,
    verbose: bool
) -> Response<Cursor<Vec<u8>>> {
    // Check if tideorm.toml exists
    if !Path::new("tideorm.toml").exists() {
        return create_json_response(json!({
            "success": false,
            "error": "No tideorm.toml found. Run 'tideorm init' first.",
        }));
    }
    
    // Read the request body
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut request.as_reader(), &mut body) {
        return create_json_response(json!({
            "success": false,
            "error": format!("Failed to read request: {}", e),
        }));
    }

    let payload: ExecuteRequest = match serde_json::from_str(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return create_json_response(json!({
                "success": false,
                "error": format!("Invalid request payload: {}", error),
            }));
        }
    };

    if payload.command.is_empty() {
        return create_json_response(json!({
            "success": false,
            "error": "No command provided",
        }));
    }

    let command_display = payload.command.join(" ");

    if verbose {
        println!("  {} Executing: tideorm {}", "→".cyan(), command_display.yellow());
    }

    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return create_json_response(json!({
                "success": false,
                "error": format!("Failed to resolve CLI executable: {}", error),
            }));
        }
    };

    let output = std::process::Command::new(executable)
        .args(&payload.command)
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

            create_json_response(json!({
                "success": success,
                "output": output_text,
            }))
        }
        Err(e) => {
            create_json_response(json!({
                "success": false,
                "error": format!("Failed to execute command: {}", e),
            }))
        }
    }
}

/// Handle SQL query execution requests
fn handle_query_request(
    request: &mut tiny_http::Request,
    verbose: bool
) -> Response<Cursor<Vec<u8>>> {
    if !Path::new("tideorm.toml").exists() {
        return create_json_response(json!({
            "success": false,
            "error": "No tideorm.toml found. Run 'tideorm init' first.",
        }));
    }

    // Read the request body
    let mut body = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut request.as_reader(), &mut body) {
        return create_json_response(json!({
            "success": false,
            "error": format!("Failed to read request: {}", e),
        }));
    }

    let payload: QueryRequest = match serde_json::from_str(&body) {
        Ok(payload) => payload,
        Err(error) => {
            return create_json_response(json!({
                "success": false,
                "error": format!("Invalid request payload: {}", error),
            }));
        }
    };

    if payload.query.trim().is_empty() {
        return create_json_response(json!({
            "success": false,
            "error": "No query provided",
        }));
    }

    if verbose {
        println!("  {} Executing query: {}", "→".cyan(), payload.query.yellow());
    }

    let config = match TideConfig::load("tideorm.toml") {
        Ok(config) => config,
        Err(error) => {
            return create_json_response(json!({
                "success": false,
                "error": error,
            }));
        }
    };

    let outcome = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            if runtime_db::formats_result_set(&payload.query) {
                let rows = runtime_db::query_json(&config, &payload.query).await?;
                serde_json::to_string_pretty(&rows).map_err(|error| error.to_string())
            } else {
                let affected = runtime_db::execute(&config, &payload.query).await?;
                Ok(format!("Query executed successfully.\nRows affected: {}", affected))
            }
        })
    });

    match outcome {
        Ok(result) => create_json_response(json!({
            "success": true,
            "result": result,
        })),
        Err(error) => create_json_response(json!({
            "success": false,
            "error": error,
        })),
    }
}
