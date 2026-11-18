//! 专用线程运行 WASM 插件宿主：加载、编译和执行都不占用 GUI 线程。

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{self, JoinHandle},
};

use fb_ext_api::PluginManifest;
use fb_ext_host::{Host, HostConfig, Invocation};

#[derive(Debug)]
pub enum PluginCommand {
    Reload {
        dir: PathBuf,
        disabled: Vec<String>,
    },
    Run {
        plugin: String,
        index: u32,
        request: u64,
    },
    Import {
        plugin: String,
        importer: String,
        path: PathBuf,
        request: u64,
    },
    SourceList {
        plugin: String,
        source: String,
        query: String,
        request: u64,
    },
    Shutdown,
}

#[derive(Debug)]
pub enum PluginEvent {
    Loaded {
        plugins: Vec<PluginSummary>,
    },
    RunResult {
        plugin: String,
        request: u64,
        result: i32,
    },
    Imported {
        plugin: String,
        request: u64,
        json: String,
    },
    SourceListed {
        plugin: String,
        source: String,
        request: u64,
        json: String,
    },
    Error {
        plugin: Option<String>,
        request: Option<u64>,
        message: String,
    },
}

/// GUI 侧需要的插件状态；manifest 与错误都可直接映射成 UI 条目。
#[derive(Clone, Debug)]
pub struct PluginSummary {
    pub manifest: PluginManifest,
    pub error: Option<String>,
    pub enabled: bool,
}

/// Cheap handle to the plugin worker thread.
pub struct PluginWorker {
    commands: Sender<PluginCommand>,
    events: Receiver<PluginEvent>,
    thread: Option<JoinHandle<()>>,
}

impl PluginWorker {
    pub fn spawn(dir: PathBuf) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let error_tx = event_tx.clone();
        let thread = thread::Builder::new()
            .name("fb-plugin-worker".into())
            .spawn(move || worker_loop(dir, command_rx, event_tx));
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(err) => {
                let _ = error_tx.send(PluginEvent::Error {
                    plugin: None,
                    request: None,
                    message: format!("插件线程启动失败：{err}"),
                });
                None
            }
        };
        Self {
            commands: command_tx,
            events: event_rx,
            thread,
        }
    }

    pub fn reload(&self, dir: PathBuf, disabled: Vec<String>) -> Result<(), String> {
        self.commands
            .send(PluginCommand::Reload { dir, disabled })
            .map_err(|_| "插件 worker 已停止".to_owned())
    }

    pub fn run(&self, plugin: String, index: u32, request: u64) -> Result<(), String> {
        self.commands
            .send(PluginCommand::Run {
                plugin,
                index,
                request,
            })
            .map_err(|_| "插件 worker 已停止".to_owned())
    }

    pub fn import(
        &self,
        plugin: String,
        importer: String,
        path: PathBuf,
        request: u64,
    ) -> Result<(), String> {
        self.commands
            .send(PluginCommand::Import {
                plugin,
                importer,
                path,
                request,
            })
            .map_err(|_| "插件 worker 已停止".to_owned())
    }

    pub fn source_list(
        &self,
        plugin: String,
        source: String,
        query: String,
        request: u64,
    ) -> Result<(), String> {
        self.commands
            .send(PluginCommand::SourceList {
                plugin,
                source,
                query,
                request,
            })
            .map_err(|_| "插件 worker 已停止".to_owned())
    }

    pub fn try_recv(&self) -> Result<PluginEvent, TryRecvError> {
        self.events.try_recv()
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// 同步等待 worker 退出；App 退出时只用 Drop 的非阻塞路径。
    #[allow(dead_code)]
    pub fn shutdown(&mut self) {
        let _ = self.commands.send(PluginCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PluginWorker {
    fn drop(&mut self) {
        // 不在 GUI 线程 join；线程收到 Shutdown 后自行释放 Host。
        let _ = self.commands.send(PluginCommand::Shutdown);
    }
}

struct WorkerPlugin {
    manifest: PluginManifest,
    invocation: Invocation,
}

struct WorkerState {
    default_dir: PathBuf,
    host: Option<Host>,
    plugins: Vec<WorkerPlugin>,
    disabled: HashSet<String>,
}

struct LoadOutcome {
    summaries: Vec<PluginSummary>,
    host_error: Option<String>,
}

impl WorkerState {
    fn new(dir: PathBuf) -> Self {
        Self {
            default_dir: dir,
            host: None,
            plugins: Vec::new(),
            disabled: HashSet::new(),
        }
    }

    fn load(&mut self, dir: &Path) -> LoadOutcome {
        self.plugins.clear();
        let host_error = if self.host.is_none() {
            match Host::new(HostConfig::default()) {
                Ok(host) => {
                    self.host = Some(host);
                    None
                }
                Err(err) => Some(format!("插件宿主不可用：{err}")),
            }
        } else {
            None
        };

        let mut summaries = Vec::new();
        let Ok(read) = fs::read_dir(dir) else {
            return LoadOutcome {
                summaries,
                host_error,
            };
        };
        let mut dirs: Vec<PathBuf> = read
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        dirs.sort();

        let mut seen_ids = HashSet::new();
        for plugin_dir in dirs {
            let fallback = plugin_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("plugin")
                .to_owned();
            let manifest = match fs::read_to_string(plugin_dir.join("plugin.json")) {
                Ok(text) => match serde_json::from_str::<PluginManifest>(&text) {
                    Ok(manifest) => manifest,
                    Err(err) => {
                        summaries.push(fallback_summary(
                            &fallback,
                            format!("plugin.json 解析失败：{err}"),
                        ));
                        continue;
                    }
                },
                Err(err) => {
                    summaries.push(fallback_summary(
                        &fallback,
                        format!("读取 plugin.json 失败：{err}"),
                    ));
                    continue;
                }
            };

            if !seen_ids.insert(manifest.id.clone()) {
                summaries.push(PluginSummary {
                    manifest,
                    error: Some("插件 id 与前面的插件重复".into()),
                    enabled: true,
                });
                continue;
            }

            if self.disabled.contains(&manifest.id) {
                summaries.push(PluginSummary {
                    manifest,
                    error: None,
                    enabled: false,
                });
                continue;
            }

            let Some(host) = self.host.as_ref() else {
                summaries.push(PluginSummary {
                    manifest,
                    error: Some(
                        host_error
                            .clone()
                            .unwrap_or_else(|| "插件宿主不可用".into()),
                    ),
                    enabled: true,
                });
                continue;
            };

            let entry = match manifest.resolve_entry(&plugin_dir) {
                Ok(entry) => entry,
                Err(err) => {
                    summaries.push(PluginSummary {
                        manifest,
                        error: Some(err.to_string()),
                        enabled: true,
                    });
                    continue;
                }
            };
            let is_wat = entry
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wat"));
            let wasm = if is_wat {
                wat::parse_file(&entry)
                    .map_err(|err| format!("解析 {} 失败：{err}", manifest.entry))
            } else {
                fs::read(&entry).map_err(|err| format!("读取 {} 失败：{err}", manifest.entry))
            };
            let wasm = match wasm {
                Ok(wasm) => wasm,
                Err(error) => {
                    summaries.push(PluginSummary {
                        manifest,
                        error: Some(error),
                        enabled: true,
                    });
                    continue;
                }
            };
            let mut invocation = match host.load(&manifest, &wasm) {
                Ok(invocation) => invocation,
                Err(err) => {
                    summaries.push(PluginSummary {
                        manifest,
                        error: Some(err.to_string()),
                        enabled: true,
                    });
                    continue;
                }
            };
            if let Err(err) = invocation.validate_command_declarations(&manifest) {
                summaries.push(PluginSummary {
                    manifest,
                    error: Some(err.to_string()),
                    enabled: true,
                });
                continue;
            }
            summaries.push(PluginSummary {
                manifest: manifest.clone(),
                error: None,
                enabled: true,
            });
            self.plugins.push(WorkerPlugin {
                manifest,
                invocation,
            });
        }

        LoadOutcome {
            summaries,
            host_error,
        }
    }

    fn run(&mut self, plugin: &str, index: u32) -> Result<i32, String> {
        let Some(loaded) = self
            .plugins
            .iter_mut()
            .find(|loaded| loaded.manifest.id == plugin)
        else {
            return Err("插件未加载或不可用".into());
        };
        if usize::try_from(index).map_or(true, |index| index >= loaded.manifest.commands.len()) {
            return Err("插件命令不存在".into());
        }
        let index = i32::try_from(index).map_err(|_| "插件命令索引超出范围".to_owned())?;
        loaded
            .invocation
            .reset_fuel()
            .map_err(|err| err.to_string())?;
        loaded
            .invocation
            .run_command(index)
            .map_err(|err| err.to_string())
    }

    fn import(&mut self, plugin: &str, importer: &str, path: &Path) -> Result<String, String> {
        let Some(loaded) = self
            .plugins
            .iter_mut()
            .find(|loaded| loaded.manifest.id == plugin)
        else {
            return Err("插件未加载或不可用".into());
        };
        if !loaded
            .manifest
            .importers
            .iter()
            .any(|entry| entry.id == importer)
        {
            return Err("插件未声明该导入器".into());
        }
        loaded
            .invocation
            .reset_fuel()
            .map_err(|err| err.to_string())?;
        loaded
            .invocation
            .import(path)
            .map_err(|err| err.to_string())
    }

    fn source_list(&mut self, plugin: &str, source: &str, query: &str) -> Result<String, String> {
        let Some(loaded) = self
            .plugins
            .iter_mut()
            .find(|loaded| loaded.manifest.id == plugin)
        else {
            return Err("插件未加载或不可用".into());
        };
        if !loaded
            .manifest
            .sources
            .iter()
            .any(|entry| entry.id == source)
        {
            return Err("插件未声明该远程源".into());
        }
        loaded
            .invocation
            .reset_fuel()
            .map_err(|err| err.to_string())?;
        loaded
            .invocation
            .source_list(source, query)
            .map_err(|err| err.to_string())
    }

    fn dir_or_default(&self, dir: PathBuf) -> PathBuf {
        if dir.as_os_str().is_empty() {
            self.default_dir.clone()
        } else {
            dir
        }
    }
}

fn worker_loop(
    default_dir: PathBuf,
    commands: Receiver<PluginCommand>,
    events: Sender<PluginEvent>,
) {
    let mut state = WorkerState::new(default_dir);
    while let Ok(command) = commands.recv() {
        match command {
            PluginCommand::Reload { dir, disabled } => {
                state.disabled = disabled
                    .into_iter()
                    .map(|id| id.trim().to_owned())
                    .filter(|id| !id.is_empty())
                    .collect();
                let dir = state.dir_or_default(dir);
                let outcome = state.load(&dir);
                if events
                    .send(PluginEvent::Loaded {
                        plugins: outcome.summaries,
                    })
                    .is_err()
                {
                    break;
                }
                if let Some(message) = outcome.host_error {
                    let _ = events.send(PluginEvent::Error {
                        plugin: None,
                        request: None,
                        message,
                    });
                }
            }
            PluginCommand::Run {
                plugin,
                index,
                request,
            } => {
                let event = match state.run(&plugin, index) {
                    Ok(result) => PluginEvent::RunResult {
                        plugin,
                        request,
                        result,
                    },
                    Err(message) => PluginEvent::Error {
                        plugin: Some(plugin),
                        request: Some(request),
                        message,
                    },
                };
                if events.send(event).is_err() {
                    break;
                }
            }
            PluginCommand::Import {
                plugin,
                importer,
                path,
                request,
            } => {
                let event = match state.import(&plugin, &importer, &path) {
                    Ok(json) => PluginEvent::Imported {
                        plugin,
                        request,
                        json,
                    },
                    Err(message) => PluginEvent::Error {
                        plugin: Some(plugin),
                        request: Some(request),
                        message,
                    },
                };
                if events.send(event).is_err() {
                    break;
                }
            }
            PluginCommand::SourceList {
                plugin,
                source,
                query,
                request,
            } => {
                let event = match state.source_list(&plugin, &source, &query) {
                    Ok(json) => PluginEvent::SourceListed {
                        plugin,
                        source,
                        request,
                        json,
                    },
                    Err(message) => PluginEvent::Error {
                        plugin: Some(plugin),
                        request: Some(request),
                        message,
                    },
                };
                if events.send(event).is_err() {
                    break;
                }
            }
            PluginCommand::Shutdown => break,
        }
    }
}

fn fallback_manifest(name: &str) -> PluginManifest {
    let version = fb_ext_api::API_VERSION.clone();
    PluginManifest {
        id: name.to_owned(),
        name: name.to_owned(),
        version: version.clone(),
        api_version: version,
        permissions: Vec::new(),
        entry: String::new(),
        integrity: None,
        commands: Vec::new(),
        panels: Vec::new(),
        importers: Vec::new(),
        sources: Vec::new(),
    }
}

fn fallback_summary(name: &str, error: String) -> PluginSummary {
    PluginSummary {
        manifest: fallback_manifest(name),
        error: Some(error),
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn copy_hello_plugin(root: &Path) {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/hello");
        let dest = root.join("hello");
        fs::create_dir_all(&dest).unwrap();
        for name in ["plugin.json", "plugin.wat"] {
            fs::copy(source.join(name), dest.join(name)).unwrap();
        }
    }

    fn recv_loaded(worker: &PluginWorker) -> Vec<PluginSummary> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match worker.try_recv() {
                Ok(PluginEvent::Loaded { plugins }) => return plugins,
                Ok(PluginEvent::Error { message, .. }) => panic!("加载失败：{message}"),
                Ok(PluginEvent::RunResult { .. }) => {}
                Ok(PluginEvent::Imported { .. }) => {}
                Ok(PluginEvent::SourceListed { .. }) => {}
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => panic!("插件 worker 已断开"),
            }
            assert!(Instant::now() < deadline, "等待插件加载超时");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn run(worker: &PluginWorker, plugin: &str, index: u32, request: u64) -> Result<i32, String> {
        worker.run(plugin.to_owned(), index, request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match worker.try_recv() {
                Ok(PluginEvent::RunResult {
                    request: seen,
                    result,
                    ..
                }) if seen == request => return Ok(result),
                Ok(PluginEvent::Error {
                    request: Some(seen),
                    message,
                    ..
                }) if seen == request => return Err(message),
                Ok(_) => {}
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => panic!("插件 worker 已断开"),
            }
            assert!(Instant::now() < deadline, "等待插件命令超时");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn loads_hello_and_runs_commands() {
        let dir = tempfile::tempdir().unwrap();
        copy_hello_plugin(dir.path());
        let worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();

        let summaries = recv_loaded(&worker);
        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert!(summary.error.is_none());
        assert_eq!(summary.manifest.commands.len(), 2);
        assert_eq!(summary.manifest.panels.len(), 1);
        assert_eq!(run(&worker, "com.example.hello", 0, 1), Ok(0));
        assert_eq!(run(&worker, "com.example.hello", 1, 2), Ok(1));
    }

    #[test]
    fn bad_manifest_and_bad_wasm_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let bad_manifest = dir.path().join("bad-manifest");
        fs::create_dir(&bad_manifest).unwrap();
        fs::write(bad_manifest.join("plugin.json"), "{not json").unwrap();

        let bad_wasm = dir.path().join("bad-wasm");
        fs::create_dir(&bad_wasm).unwrap();
        fs::write(
            bad_wasm.join("plugin.json"),
            r#"{
                "id": "com.example.bad",
                "name": "Bad",
                "version": "0.1.0",
                "api_version": "0.1.0",
                "permissions": ["register_commands", "declare_panels"],
                "entry": "plugin.wasm",
                "commands": [{ "id": "bad.run", "title": "Run" }],
                "panels": [{ "id": "bad.panel", "title": "Panel" }]
            }"#,
        )
        .unwrap();
        fs::write(bad_wasm.join("plugin.wasm"), b"not wasm").unwrap();

        let worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();
        let summaries = recv_loaded(&worker);
        assert_eq!(summaries.len(), 2);

        let manifest_error = summaries
            .iter()
            .find(|summary| summary.manifest.id == "bad-manifest")
            .expect("坏 manifest 应有错误摘要");
        assert!(manifest_error.error.is_some());
        assert!(manifest_error.manifest.commands.is_empty());

        let wasm_error = summaries
            .iter()
            .find(|summary| summary.manifest.id == "com.example.bad")
            .expect("坏 wasm 应保留 manifest");
        assert!(wasm_error.error.is_some());
        assert_eq!(wasm_error.manifest.commands.len(), 1);
        assert_eq!(wasm_error.manifest.panels.len(), 1);
    }

    #[test]
    fn disabled_plugin_skips_wasm_and_rejects_run() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("disabled");
        fs::create_dir(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.json"),
            r#"{
                "id": "com.example.disabled",
                "name": "Disabled",
                "version": "0.1.0",
                "api_version": "0.1.0",
                "permissions": ["register_commands"],
                "entry": "plugin.wasm",
                "commands": [{ "id": "disabled.run", "title": "Run" }]
            }"#,
        )
        .unwrap();
        // 坏 wasm：启用时应该报错；禁用时不应尝试加载。
        fs::write(plugin.join("plugin.wasm"), b"not wasm").unwrap();

        let worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker
            .reload(
                dir.path().to_path_buf(),
                vec!["com.example.disabled".into()],
            )
            .unwrap();
        let summaries = recv_loaded(&worker);
        assert_eq!(summaries.len(), 1);
        assert!(!summaries[0].enabled);
        assert!(summaries[0].error.is_none());
        assert_eq!(
            run(&worker, "com.example.disabled", 0, 1),
            Err("插件未加载或不可用".into())
        );

        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();
        let summaries = recv_loaded(&worker);
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].enabled);
        assert!(summaries[0].error.is_some());
    }

    #[test]
    fn duplicate_ids_reject_second_plugin() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["one", "two"] {
            let plugin = dir.path().join(name);
            fs::create_dir(&plugin).unwrap();
            fs::write(
                plugin.join("plugin.json"),
                r#"{
                    "id": "com.example.duplicate",
                    "name": "Duplicate",
                    "version": "0.1.0",
                    "api_version": "0.1.0",
                    "permissions": ["register_commands"],
                    "entry": "plugin.wat",
                    "commands": [{ "id": "dup.run", "title": "Run" }]
                }"#,
            )
            .unwrap();
            fs::write(
                plugin.join("plugin.wat"),
                r#"(module
                    (func (export "fb_command_count") (result i32) (i32.const 1))
                    (func (export "fb_command_run") (param i32) (result i32) (i32.const 0))
                )"#,
            )
            .unwrap();
        }

        let worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();
        let summaries = recv_loaded(&worker);
        assert_eq!(summaries.len(), 2);
        assert_eq!(
            summaries
                .iter()
                .filter(|summary| summary.error.is_none())
                .count(),
            1
        );
        assert_eq!(
            summaries
                .iter()
                .filter(|summary| summary.error.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn rejects_entry_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("escape");
        fs::create_dir(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.json"),
            r#"{
                "id": "com.example.escape",
                "name": "Escape",
                "version": "0.1.0",
                "api_version": "0.1.0",
                "entry": "../evil.wasm"
            }"#,
        )
        .unwrap();

        let worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();
        let summaries = recv_loaded(&worker);
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].error.is_some());
    }

    #[test]
    fn reload_does_not_accumulate_plugins() {
        let dir = tempfile::tempdir().unwrap();
        copy_hello_plugin(dir.path());
        let worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();
        assert_eq!(recv_loaded(&worker).len(), 1);
        worker.reload(dir.path().to_path_buf(), Vec::new()).unwrap();
        assert_eq!(recv_loaded(&worker).len(), 1);
    }

    #[test]
    fn missing_directory_loads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let worker = PluginWorker::spawn(missing.clone());
        worker.reload(missing, Vec::new()).unwrap();
        assert!(recv_loaded(&worker).is_empty());
    }

    #[test]
    fn finished_worker_rejects_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut worker = PluginWorker::spawn(dir.path().to_path_buf());
        worker.shutdown();
        assert!(worker.is_finished());
        assert!(worker.reload(dir.path().to_path_buf(), Vec::new()).is_err());
        assert!(worker.run("com.example.hello".into(), 0, 1).is_err());
        assert!(matches!(worker.try_recv(), Err(TryRecvError::Disconnected)));
    }
}
