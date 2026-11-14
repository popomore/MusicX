//! Explicit, payload-bound helper permission and scoped child lifetimes.
use crate::{
    mygo::{self, Manifest},
    plugin_files,
};
use anyhow::{Context, Result, ensure};
use musicx_core::{protocol::tr, storage};
use rquickjs::{Ctx, Function};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Instant,
};

fn approval_name(id: &str) -> String {
    format!("mygo-{id}-process-permission.json")
}
pub(crate) fn programs(manifest: &Manifest) -> Vec<String> {
    match &manifest.permissions.process {
        Value::Bool(true) => vec!["*".into()],
        Value::Array(commands) => commands
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}
pub(crate) fn granted(manifest: &Manifest) -> bool {
    if cfg!(target_os = "ios") || programs(manifest).is_empty() {
        return false;
    }
    let check = (|| -> Result<bool> {
        let root = mygo::plugin_root()?.join(&manifest.id);
        let path = storage::state_dir()?.join(approval_name(&manifest.id));
        let file = fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024).read_to_end(&mut bytes)?;
        let saved: BTreeMap<String, String> = serde_json::from_slice(&bytes)?;
        Ok(saved == plugin_files::hashes(&root)?)
    })();
    check.unwrap_or(false)
}
pub(crate) fn set(id: &str, enabled: bool) -> Result<()> {
    ensure!(mygo::valid_id(id), tr!("Invalid plugin ID"));
    let root = mygo::plugin_root()?.join(id);
    let manifest = mygo::manifest(&root)?;
    ensure!(
        !enabled || (!cfg!(target_os = "ios") && !programs(&manifest).is_empty()),
        tr!("This source has no external program permission")
    );
    if enabled {
        storage::save_private(
            &approval_name(id),
            &serde_json::to_string(&plugin_files::hashes(&root)?)?,
        )?;
    } else {
        let path = storage::state_dir()?.join(approval_name(id));
        if let Err(error) = fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
        for runtime in runtimes()
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|r| r.id == id)
        {
            runtime.kill_all();
        }
    }
    Ok(())
}
fn runtimes() -> &'static Mutex<Vec<Weak<Runtime>>> {
    static R: std::sync::OnceLock<Mutex<Vec<Weak<Runtime>>>> = std::sync::OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}
static CHILDREN: AtomicUsize = AtomicUsize::new(0);
struct Slot;
impl Slot {
    fn acquire() -> Result<Self> {
        let previous = CHILDREN.fetch_add(1, Ordering::AcqRel);
        if previous >= 16 {
            CHILDREN.fetch_sub(1, Ordering::AcqRel);
            anyhow::bail!(tr!("External program limit reached"));
        }
        Ok(Self)
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        CHILDREN.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Output {
    lines: VecDeque<String>,
    eof: bool,
    error: Option<String>,
    stderr: Vec<u8>,
}
struct Running {
    child: Mutex<Child>,
    input: mpsc::SyncSender<Option<Vec<u8>>>,
    output: Arc<Mutex<Output>>,
    slot: Mutex<Option<Slot>>,
    cleaned: AtomicBool,
}
impl Running {
    fn clean_group(&self, pid: u32) {
        if !self.cleaned.swap(true, Ordering::AcqRel) {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            self.slot.lock().unwrap().take();
        }
    }
    fn status(&self) -> Result<Option<std::process::ExitStatus>> {
        let mut child = self.child.lock().unwrap();
        let status = child.try_wait()?;
        if status.is_some() {
            self.clean_group(child.id());
        }
        Ok(status)
    }
    fn kill(&self) {
        let mut child = self.child.lock().unwrap();
        // Clean descendants even if their direct parent has already exited.
        self.clean_group(child.id());
        let _ = child.wait();
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.kill();
    }
}
pub(crate) struct Runtime {
    id: String,
    root: PathBuf,
    allowed: Vec<String>,
    children: Mutex<Vec<Arc<Running>>>,
}
impl Runtime {
    pub(crate) fn new(manifest: &Manifest) -> Result<Option<Arc<Self>>> {
        if !granted(manifest) {
            return Ok(None);
        }
        let runtime = Arc::new(Self {
            id: manifest.id.clone(),
            root: mygo::plugin_root()?.join(&manifest.id).canonicalize()?,
            allowed: programs(manifest),
            children: Mutex::new(Vec::new()),
        });
        let mut all = runtimes().lock().unwrap();
        all.retain(|r| r.strong_count() > 0);
        all.push(Arc::downgrade(&runtime));
        Ok(Some(runtime))
    }
    fn kill_all(&self) {
        for child in self.children.lock().unwrap().iter() {
            child.kill();
        }
    }
    pub(crate) fn has_children(&self) -> bool {
        !self.children.lock().unwrap().is_empty()
    }
    fn program(&self, name: &str) -> Result<PathBuf> {
        ensure!(
            !name.is_empty() && name.len() <= 4096 && !name.contains('\0') && !name.contains('\\'),
            tr!("Invalid external program")
        );
        let path = PathBuf::from(name);
        let relative = path.is_relative() && name.contains('/');
        let local = path
            .strip_prefix(&self.root)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or(name);
        let matched = self.allowed.iter().any(|rule| {
            rule == "*"
                || rule == name
                || rule == local
                || rule.strip_suffix('*').is_some_and(|prefix| {
                    [name, local]
                        .iter()
                        .any(|s| s.starts_with(prefix) && !s[prefix.len()..].contains('/'))
                })
        });
        ensure!(
            matched,
            tr!("External program is not declared by this source")
        );
        if relative {
            ensure!(
                plugin_files::path_allowed(name),
                tr!("Invalid external program")
            );
            let path = self.root.join(name).canonicalize()?;
            ensure!(
                path.starts_with(&self.root) && path.is_file(),
                tr!("Invalid external program")
            );
            Ok(path)
        } else {
            Ok(path)
        }
    }
    fn spawn(&self, text: &str, deadline: Instant) -> Result<Value> {
        ensure!(
            text.len() <= 2 * 1024 * 1024 && Instant::now() < deadline,
            tr!("External program request is too large or expired")
        );
        let value: Value = serde_json::from_str(text)?;
        let program = value["program"]
            .as_str()
            .context(tr!("Invalid external program"))?;
        let args: Vec<String> =
            serde_json::from_value(value.get("args").cloned().unwrap_or(json!([])))?;
        ensure!(
            args.len() <= 256 && args.iter().all(|v| !v.contains('\0')),
            tr!("Invalid external program arguments")
        );
        // Recheck revocation and on-disk edits before each launch.
        ensure!(
            granted(&mygo::manifest(&self.root)?),
            tr!("Allow external programs in this source's settings first")
        );
        let mut children = self.children.lock().unwrap();
        ensure!(
            children.len() < 64
                && children
                    .iter()
                    .filter(|child| child.status().is_ok_and(|s| s.is_none()))
                    .count()
                    < 4,
            tr!("External program limit reached")
        );
        let slot = Slot::acquire()?;
        let mut command = Command::new(self.program(program)?);
        command
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let mut child = command
            .spawn()
            .with_context(|| tr!("Could not start external program {}", program))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let pid = child.id();
        let output = Arc::new(Mutex::new(Output {
            lines: VecDeque::new(),
            eof: false,
            error: None,
            stderr: Vec::new(),
        }));
        let (send, receive) = mpsc::sync_channel::<Option<Vec<u8>>>(8);
        std::thread::spawn(move || {
            while let Ok(Some(bytes)) = receive.recv() {
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let running = Arc::new(Running {
            child: Mutex::new(child),
            input: send,
            output: output.clone(),
            slot: Mutex::new(Some(slot)),
            cleaned: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&running);
        let out = output.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout.take(8 * 1024 * 1024 + 1));
            let mut total = 0usize;
            loop {
                let mut line = Vec::new();
                let read = reader.read_until(b'\n', &mut line);
                let mut state = out.lock().unwrap();
                match read {
                    Ok(0) => {
                        state.eof = true;
                        break;
                    }
                    Ok(n) => {
                        total += n;
                        if total > 8 * 1024 * 1024 || state.lines.len() >= 256 {
                            state.error =
                                Some(tr!("External program output exceeds the size limit").into());
                            state.eof = true;
                            drop(state);
                            if let Some(child) = weak.upgrade() {
                                child.kill();
                            }
                            break;
                        }
                        while matches!(line.last(), Some(b'\n' | b'\r')) {
                            line.pop();
                        }
                        state
                            .lines
                            .push_back(String::from_utf8_lossy(&line).into_owned());
                    }
                    Err(_) => {
                        state.eof = true;
                        state.error = Some(tr!("Could not read external program output").into());
                        break;
                    }
                }
            }
        });
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut bytes = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                let mut state = output.lock().unwrap();
                let remaining = (64 * 1024usize).saturating_sub(state.stderr.len());
                state.stderr.extend_from_slice(&bytes[..n.min(remaining)]);
            }
        });
        let id = children.len();
        children.push(running);
        Ok(json!({"handle":id,"pid":pid}))
    }
    fn operation(&self, text: &str) -> Result<Value> {
        ensure!(
            text.len() <= 2 * 1024 * 1024,
            tr!("External program request is too large or expired")
        );
        let value: Value = serde_json::from_str(text)?;
        let id = value["handle"]
            .as_u64()
            .context(tr!("Invalid external program handle"))? as usize;
        let running = self
            .children
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context(tr!("Invalid external program handle"))?;
        Ok(match value["op"].as_str().unwrap_or_default() {
            "write" => {
                let bytes = value["text"]
                    .as_str()
                    .context(tr!("Invalid external program arguments"))?
                    .as_bytes()
                    .to_vec();
                running
                    .input
                    .try_send(Some(bytes))
                    .context(tr!("External program input is busy or closed"))?;
                json!({})
            }
            "close" => {
                running
                    .input
                    .try_send(None)
                    .context(tr!("External program input is busy or closed"))?;
                json!({})
            }
            "read" => {
                let mut out = running.output.lock().unwrap();
                if let Some(error) = &out.error {
                    anyhow::bail!("{error}");
                }
                if let Some(line) = out.lines.pop_front() {
                    json!({"line":line})
                } else if out.eof {
                    json!({"line":null})
                } else {
                    json!({"pending":true})
                }
            }
            "wait" => {
                let status = running.status()?;
                match status {
                    Some(status) => json!({"status":status.code().unwrap_or(-1)}),
                    None => json!({"pending":true}),
                }
            }
            "stderr" => {
                json!({"text":String::from_utf8_lossy(&running.output.lock().unwrap().stderr)})
            }
            "kill" => {
                running.kill();
                json!({})
            }
            _ => anyhow::bail!(tr!("Invalid external program operation")),
        })
    }
    pub(crate) fn install(
        self: &Arc<Self>,
        ctx: &Ctx<'_>,
        deadline: Instant,
    ) -> rquickjs::Result<()> {
        let runtime = self.clone();
        ctx.globals().set(
            "__processSpawn",
            Function::new(ctx.clone(), move |text: String| {
                runtime
                    .spawn(&text, deadline)
                    .unwrap_or_else(|e| json!({"error":e.to_string()}))
                    .to_string()
            })?,
        )?;
        let runtime = self.clone();
        ctx.globals().set(
            "__processOp",
            Function::new(ctx.clone(), move |text: String| {
                runtime
                    .operation(&text)
                    .unwrap_or_else(|e| json!({"error":e.to_string()}))
                    .to_string()
            })?,
        )?;
        Ok(())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.kill_all();
    }
}

pub(crate) fn install_paths(ctx: &Ctx<'_>, manifest: &Manifest) -> Result<()> {
    let root = mygo::plugin_root()?.join(&manifest.id).canonicalize()?;
    let data = storage::state_dir()?.join("mygo-data").join(&manifest.id);
    fs::create_dir_all(&data)?;
    ensure!(
        !fs::symlink_metadata(&data)?.file_type().is_symlink(),
        tr!("Invalid source data directory")
    );
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700))?;
    ctx.globals()
        .set("__pluginDir", root.to_string_lossy().as_ref())?;
    ctx.globals()
        .set("__dataDir", data.to_string_lossy().as_ref())?;
    let allowed = granted(manifest);
    ctx.globals().set(
        "__removeDataDir",
        Function::new(ctx.clone(), move || {
            if !allowed {
                return false;
            }
            // remove_dir_all does not follow directory symlinks.
            fs::remove_dir_all(&data)
                .and_then(|_| fs::create_dir(&data))
                .and_then(|_| fs::set_permissions(&data, fs::Permissions::from_mode(0o700)))
                .is_ok()
        })?,
    )?;
    Ok(())
}
