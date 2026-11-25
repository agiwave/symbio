use std::path::{Path, PathBuf};

use super::id::validate_session_id;
use super::{run_worker, BuildId, Epoch, WorkerBootstrap, WorkerId};

pub async fn run_worker_command<I, S>(argv: I) -> Option<u8>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut argv = argv.into_iter().map(Into::into);
    if argv.next().as_deref() != Some("__worker") {
        return None;
    }
    let code = match parse_worker(argv) {
        Ok(bootstrap) => match run_worker(bootstrap).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("worker 退出: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("worker 参数错误: {error}");
            2
        }
    };
    Some(code)
}

fn parse_worker<I: IntoIterator<Item = String>>(argv: I) -> Result<WorkerBootstrap, String> {
    let cwd = std::env::current_dir().map_err(|error| format!("无法获取当前目录: {error}"))?;
    let mut homedir = std::env::var_os("SYMBIO_HOMEDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(".symbio"));
    let mut session = String::new();
    let mut session_dir = PathBuf::new();
    let mut epoch = None;
    let mut build = None;
    let mut argv = argv.into_iter();
    while let Some(flag) = argv.next() {
        let value = argv
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{flag} 缺少取值"))?;
        match flag.as_str() {
            "--session" => session = value,
            "--session-dir" => session_dir = resolve_path(&value, &cwd),
            "--epoch" => {
                let value = value.parse::<u64>().map_err(|_| "epoch 必须是整数")?;
                epoch = Epoch::new(value);
            }
            "--build" => build = Some(value),
            "--homedir" => homedir = resolve_path(&value, &cwd),
            other => return Err(format!("未知 worker 参数: {other}")),
        }
    }
    if session.is_empty()
        || session_dir.as_os_str().is_empty()
        || epoch.is_none()
        || build.as_deref().is_none_or(str::is_empty)
    {
        return Err("worker 缺少 session / session-dir / epoch / build".to_string());
    }
    let epoch = epoch.ok_or_else(|| "worker 缺少 epoch".to_string())?;
    let build = build.ok_or_else(|| "worker 缺少 build".to_string())?;
    validate_session_id(&session).map_err(|error| error.to_string())?;
    let build = BuildId::parse(build).map_err(|error| error.to_string())?;
    let worker = WorkerId::new(session, epoch, build).map_err(|error| error.to_string())?;
    Ok(WorkerBootstrap {
        worker,
        homedir,
        session_dir,
    })
}

fn resolve_path(value: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
#[path = "entry.test.rs"]
mod tests;
