//! Opt-in measurements for `bench/`. With `FM_BENCH=<file>` set, the app writes events
//! there as JSON lines (milliseconds since the process was created, memory counters)
//! and runs the steps listed in the file `FM_BENCH_STEPS` names (`app::bench`). Unset,
//! nothing is recorded and nothing runs.

use std::fs::File;
use std::io::Write as _;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

static OUT: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn out() -> Option<&'static Mutex<File>> {
    OUT.get_or_init(|| std::env::var_os("FM_BENCH").and_then(|p| File::create(p).ok()).map(Mutex::new)).as_ref()
}

pub fn enabled() -> bool {
    out().is_some()
}

/// Appends `{"event": name, "t_ms": …}` plus `fields` (an object).
pub fn event(name: &str, fields: Value) {
    let Some(out) = out() else { return };
    let mut line = json!({ "event": name, "t_ms": since_start_ms() });
    if let (Some(line), Value::Object(fields)) = (line.as_object_mut(), fields) {
        line.extend(fields);
    }
    if let Ok(mut file) = out.lock() {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// Milliseconds since the OS created the process, so loading the exe counts too.
pub fn since_start_ms() -> f64 {
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user).is_err() {
            return 0.;
        }
        ticks(GetSystemTimePreciseAsFileTime()).saturating_sub(ticks(created)) as f64 / 10_000.
    }
}

/// Working set and private bytes in MB: the counters Task Manager and `Get-Process` show.
pub fn memory() -> Value {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let read = unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS, size)
    };
    if read.is_err() {
        return json!({});
    }
    let mb = |bytes: usize| bytes as f64 / (1024. * 1024.);
    json!({ "working_set_mb": mb(counters.WorkingSetSize), "private_mb": mb(counters.PrivateUsage) })
}
