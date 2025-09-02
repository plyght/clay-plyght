use anyhow::{Result, anyhow};
use console::style;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, broadcast};
use tokio::time::{Duration, Instant, sleep};

use crate::bundler::Bundler;
use crate::cli_style::CliStyle;

pub struct DevServer {
    port: u16,
    host: String,
    public_dir: PathBuf,
    bundle_cache: Arc<RwLock<Option<String>>>,
    file_watcher: Arc<RwLock<FileWatcher>>,
    ws_clients: Arc<RwLock<Vec<broadcast::Sender<String>>>>,
}

struct FileWatcher {
    watched_files: HashMap<PathBuf, Instant>,
    last_check: Instant,
}

impl FileWatcher {
    fn new() -> Self {
        Self {
            watched_files: HashMap::new(),
            last_check: Instant::now(),
        }
    }

    async fn check_for_changes(&mut self, watch_paths: &[PathBuf]) -> Result<bool> {
        let mut has_changes = false;
        let now = Instant::now();

        for path in watch_paths {
            if let Ok(metadata) = fs::metadata(path).await {
                if let Ok(modified) = metadata.modified() {
                    let modified_instant = Instant::now()
                        - Duration::from_secs(modified.elapsed().unwrap_or_default().as_secs());

                    match self.watched_files.get(path) {
                        Some(last_modified) => {
                            if modified_instant > *last_modified {
                                has_changes = true;
                                self.watched_files.insert(path.clone(), modified_instant);
                            }
                        }
                        None => {
                            self.watched_files.insert(path.clone(), modified_instant);
                            if now.duration_since(self.last_check) > Duration::from_millis(100) {
                                has_changes = true;
                            }
                        }
                    }
                }
            }
        }

        self.last_check = now;
        Ok(has_changes)
    }

    fn add_watched_paths(&mut self, paths: Vec<PathBuf>) {
        let now = Instant::now();
        for path in paths {
            self.watched_files.entry(path).or_insert(now);
        }
    }
}

impl DevServer {
    pub fn new() -> Self {
        Self {
            port: 3000,
            host: "localhost".to_string(),
            public_dir: PathBuf::from("public"),
            bundle_cache: Arc::new(RwLock::new(None)),
            file_watcher: Arc::new(RwLock::new(FileWatcher::new())),
            ws_clients: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn start(&mut self, host: &str, port: u16) -> Result<()> {
        self.host = host.to_string();
        self.port = port;

        let server_spinner = CliStyle::create_spinner(&format!(
            "Starting development server on {host}:{port}..."
        ));

        // Initial bundle
        server_spinner.set_message("Building initial bundle...");
        self.rebuild_bundle().await?;

        // Start file watcher
        server_spinner.set_message("Starting file watcher...");
        let file_watcher = Arc::clone(&self.file_watcher);
        let bundle_cache = Arc::clone(&self.bundle_cache);
        let ws_clients = Arc::clone(&self.ws_clients);

        tokio::spawn(async move {
            Self::watch_files(file_watcher, bundle_cache, ws_clients).await;
        });

        // Start HTTP server
        server_spinner.set_message("Starting HTTP server...");
        let listener = TcpListener::bind(format!("{host}:{port}")).await?;

        server_spinner.finish_with_message(format!(
            "Server running at {}",
            style(&format!("http://{host}:{port}")).cyan().underlined()
        ));

        while let Ok((stream, addr)) = listener.accept().await {
            println!("{} Connection from {}", style("→").dim(), addr);

            let bundle_cache = Arc::clone(&self.bundle_cache);
            let public_dir = self.public_dir.clone();
            let ws_clients = Arc::clone(&self.ws_clients);

            tokio::spawn(async move {
                if let Err(e) =
                    Self::handle_connection(stream, bundle_cache, public_dir, ws_clients).await
                {
                    eprintln!("Error handling connection: {e}");
                }
            });
        }

        Ok(())
    }

    async fn rebuild_bundle(&self) -> Result<()> {
        let rebuild_spinner = CliStyle::create_spinner("Rebuilding bundle...");
        let start_time = Instant::now();

        let mut bundler = Bundler::new();
        let bundle_output = std::env::temp_dir().join("clay_dev_bundle.js");

        bundler
            .bundle(Some(bundle_output.to_str().unwrap()), false, false)
            .await?;

        rebuild_spinner.set_message("Injecting HMR client...");
        let bundle_content = fs::read_to_string(&bundle_output).await?;
        let bundle_with_hmr = self.inject_hmr_client(&bundle_content);

        {
            let mut cache = self.bundle_cache.write().await;
            *cache = Some(bundle_with_hmr);
        }

        let duration = start_time.elapsed();
        rebuild_spinner.finish_with_message(format!(
            "Bundle rebuilt in {}",
            CliStyle::format_duration(duration)
        ));

        // Notify connected clients
        self.notify_clients("reload").await;

        Ok(())
    }

    fn inject_hmr_client(&self, bundle_content: &str) -> String {
        let hmr_client = format!(
            r#"
// Clay HMR Client
(function() {{
  const ws = new WebSocket('ws://{}:{}/ws');
  
  ws.onmessage = function(event) {{
    const message = JSON.parse(event.data);
    
    if (message.type === 'reload') {{
      console.log('[Clay HMR] Reloading...');
      window.location.reload();
    }} else if (message.type === 'update') {{
      console.log('[Clay HMR] Hot update received');
      // Handle hot module replacement here
    }}
  }};
  
  ws.onopen = function() {{
    console.log('[Clay HMR] Connected to dev server');
  }};
  
  ws.onerror = function(error) {{
    console.error('[Clay HMR] WebSocket error:', error);
  }};
}})();

"#,
            self.host, self.port
        );

        format!("{hmr_client}\n{bundle_content}")
    }

    async fn watch_files(
        file_watcher: Arc<RwLock<FileWatcher>>,
        bundle_cache: Arc<RwLock<Option<String>>>,
        ws_clients: Arc<RwLock<Vec<broadcast::Sender<String>>>>,
    ) {
        let watch_paths = Self::get_watch_paths().await;

        {
            let mut watcher = file_watcher.write().await;
            watcher.add_watched_paths(watch_paths.clone());
        }

        loop {
            sleep(Duration::from_millis(500)).await;

            let has_changes = {
                let mut watcher = file_watcher.write().await;
                watcher
                    .check_for_changes(&watch_paths)
                    .await
                    .unwrap_or(false)
            };

            if has_changes {
                println!(
                    "{} File changes detected, rebuilding...",
                    CliStyle::info("File changes detected, rebuilding...")
                );

                match Self::rebuild_bundle_static(bundle_cache.clone()).await {
                    Ok(()) => {
                        Self::notify_clients_static(ws_clients.clone(), "reload").await;
                    }
                    Err(e) => {
                        println!("{}", CliStyle::error(&format!("Build error: {e}")));
                        Self::notify_clients_static(ws_clients.clone(), &format!("error:{e}"))
                            .await;
                    }
                }
            }
        }
    }

    async fn get_watch_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Watch common source directories
        let watch_dirs = vec!["src", "lib", "components"];

        for dir in watch_dirs {
            if let Ok(entries) = Self::collect_files_recursively(dir).await {
                paths.extend(entries);
            }
        }

        // Also watch package.json
        if PathBuf::from("package.json").exists() {
            paths.push(PathBuf::from("package.json"));
        }

        paths
    }

    async fn collect_files_recursively(dir: &str) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let path = PathBuf::from(dir);

        if !path.exists() || !path.is_dir() {
            return Ok(files);
        }

        let mut stack = vec![path];

        while let Some(current_path) = stack.pop() {
            let mut entries = fs::read_dir(&current_path).await?;

            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();

                if entry_path.is_dir() {
                    stack.push(entry_path);
                } else if let Some(ext) = entry_path.extension() {
                    if matches!(
                        ext.to_str(),
                        Some("js") | Some("ts") | Some("jsx") | Some("tsx")
                    ) {
                        files.push(entry_path);
                    }
                }
            }
        }

        Ok(files)
    }

    async fn rebuild_bundle_static(bundle_cache: Arc<RwLock<Option<String>>>) -> Result<()> {
        let mut bundler = Bundler::new();
        let bundle_output = std::env::temp_dir().join("clay_dev_bundle.js");

        bundler
            .bundle(Some(bundle_output.to_str().unwrap()), false, false)
            .await?;
        let bundle_content = fs::read_to_string(&bundle_output).await?;

        {
            let mut cache = bundle_cache.write().await;
            *cache = Some(bundle_content);
        }

        Ok(())
    }

    async fn notify_clients(&self, message_type: &str) {
        Self::notify_clients_static(Arc::clone(&self.ws_clients), message_type).await;
    }

    async fn notify_clients_static(
        ws_clients: Arc<RwLock<Vec<broadcast::Sender<String>>>>,
        message_type: &str,
    ) {
        let message = json!({
            "type": message_type,
            "timestamp": chrono::Utc::now().timestamp()
        })
        .to_string();

        let clients = ws_clients.read().await;
        for client in clients.iter() {
            let _ = client.send(message.clone());
        }
    }

    async fn handle_connection(
        mut stream: TcpStream,
        bundle_cache: Arc<RwLock<Option<String>>>,
        public_dir: PathBuf,
        ws_clients: Arc<RwLock<Vec<broadcast::Sender<String>>>>,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        // Read the first line to get the HTTP request
        let mut buf = [0; 1024];
        let n = stream.peek(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);
        let request_line = request.lines().next().unwrap_or("").to_string();

        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(anyhow!("Invalid HTTP request"));
        }

        let method = parts[0];
        let path = parts[1];

        println!("{} {} {}", style("→").dim(), method, path);

        // Handle WebSocket upgrade for HMR
        if path == "/ws" {
            return Self::handle_websocket_upgrade(stream, ws_clients).await;
        }

        // Serve bundle.js
        if path == "/bundle.js" {
            let bundle = {
                let cache = bundle_cache.read().await;
                cache
                    .clone()
                    .unwrap_or_else(|| "// Bundle not ready".to_string())
            };

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\n\r\n{}",
                bundle.len(),
                bundle
            );

            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }

        // Serve static files
        let file_path = if path == "/" {
            public_dir.join("index.html")
        } else {
            public_dir.join(&path[1..]) // Remove leading slash
        };

        if file_path.exists() {
            let content = fs::read(&file_path).await?;
            let content_type = Self::get_content_type(&file_path);

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
                content_type,
                content.len()
            );

            stream.write_all(response.as_bytes()).await?;
            stream.write_all(&content).await?;
        } else {
            // Serve default HTML for SPA routing
            let default_html = Self::get_default_html();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                default_html.len(),
                default_html
            );

            stream.write_all(response.as_bytes()).await?;
        }

        Ok(())
    }

    async fn handle_websocket_upgrade(
        _stream: TcpStream,
        ws_clients: Arc<RwLock<Vec<broadcast::Sender<String>>>>,
    ) -> Result<()> {
        // Simple WebSocket implementation would go here
        // For now, we'll just add a mock client
        let (tx, _rx) = broadcast::channel(100);
        {
            let mut clients = ws_clients.write().await;
            clients.push(tx);
        }

        Ok(())
    }

    fn get_content_type(path: &Path) -> &'static str {
        match path.extension().and_then(|s| s.to_str()) {
            Some("html") => "text/html",
            Some("js") => "application/javascript",
            Some("css") => "text/css",
            Some("json") => "application/json",
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("svg") => "image/svg+xml",
            _ => "text/plain",
        }
    }

    fn get_default_html() -> String {
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Clay Dev Server</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            margin: 0;
            padding: 20px;
            background: #f5f5f5;
        }
        .container {
            max-width: 800px;
            margin: 0 auto;
            background: white;
            padding: 40px;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }
        .logo {
            font-size: 24px;
            font-weight: bold;
            color: #2563eb;
            margin-bottom: 20px;
        }
        .status {
            padding: 12px;
            background: #dbeafe;
            border: 1px solid #3b82f6;
            border-radius: 4px;
            margin-bottom: 20px;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="logo">🏺 Clay Dev Server</div>
        <div class="status">
            Development server is running. Your bundle will appear here once built.
        </div>
        <div id="app"></div>
    </div>
    <script src="/bundle.js"></script>
</body>
</html>"#
            .to_string()
    }
}

impl Default for DevServer {
    fn default() -> Self {
        Self::new()
    }
}
