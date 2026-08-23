use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::thread;
use std::time::{Duration, Instant};

const MODE: &str = "LATTICE_CODEX_TEST_MODE";
const DESCENDANT_TRIGGER: &str = "LATTICE_CODEX_TEST_DESCENDANT_TRIGGER";
const DESCENDANT_DEADLINE: &str = "LATTICE_CODEX_TEST_DESCENDANT_DEADLINE";
const DESCENDANT_EFFECT: &str = "LATTICE_CODEX_TEST_DESCENDANT_EFFECT";
const NATIVE_MODE_FILE: &str = "native-process-mode.txt";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::from(2),
    }
}

fn run() -> Result<(), ()> {
    if env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--descendant")) {
        return run_descendant();
    }
    if env::args_os().skip(1).collect::<Vec<_>>()
        != vec![
            std::ffi::OsString::from("app-server"),
            std::ffi::OsString::from("--listen"),
            std::ffi::OsString::from("stdio://"),
        ]
    {
        return Err(());
    }
    run_app_server()
}

fn run_app_server() -> Result<(), ()> {
    let codex_home = required("CODEX_HOME")?;
    let root = PathBuf::from(&codex_home).parent().ok_or(())?.to_path_buf();
    let mode = fs::read_to_string(root.join(NATIVE_MODE_FILE))
        .map_err(|_| ())?
        .trim()
        .to_owned();
    let mut input = io::BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();
    read_line(&mut input)?;
    let reported_home = if mode == "wrong-home" {
        root.join("wrong-home").display().to_string()
    } else {
        codex_home.clone()
    };
    writeln!(
        output,
        "{{\"id\":0,\"result\":{{\"userAgent\":\"codex_cli_rs/0.144.6\",\"platformFamily\":\"windows\",\"platformOs\":\"windows\",\"codexHome\":{}}}}}",
        json_string(&reported_home)
    )
    .map_err(|_| ())?;
    output.flush().map_err(|_| ())?;
    if mode == "malformed" {
        writeln!(output, "{{not-json").map_err(|_| ())?;
        output.flush().map_err(|_| ())?;
        thread::sleep(Duration::from_secs(60));
        return Ok(());
    }
    if mode == "eof" || mode == "wrong-home" {
        return Ok(());
    }
    if mode == "premature" {
        writeln!(
            output,
            "{{\"id\":1,\"result\":{{\"thread\":{{\"id\":\"thread-scripted\"}}}}}}"
        )
        .map_err(|_| ())?;
        writeln!(
            output,
            "{{\"id\":2,\"result\":{{\"turn\":{{\"id\":\"turn-scripted\"}}}}}}"
        )
        .map_err(|_| ())?;
        writeln!(output, "{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"thread-scripted\",\"turn\":{{\"id\":\"turn-scripted\",\"items\":[],\"status\":\"completed\",\"error\":null}}}}}}").map_err(|_| ())?;
        output.flush().map_err(|_| ())?;
        thread::sleep(Duration::from_secs(60));
        return Ok(());
    }
    read_line(&mut input)?;
    fs::write(root.join("thread-started.txt"), b"thread/start received").map_err(|_| ())?;
    read_line(&mut input)?;
    writeln!(
        output,
        "{{\"id\":1,\"result\":{{\"thread\":{{\"id\":\"thread-scripted\"}}}}}}"
    )
    .map_err(|_| ())?;
    output.flush().map_err(|_| ())?;
    read_line(&mut input)?;
    writeln!(
        output,
        "{{\"id\":2,\"result\":{{\"turn\":{{\"id\":\"turn-scripted\"}}}}}}"
    )
    .map_err(|_| ())?;
    output.flush().map_err(|_| ())?;

    match mode.as_str() {
        "success" => {
            write_completed(&mut output)?;
            thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "yielded" => {
            writeln!(output, r#"{{"method":"item/completed","params":{{"threadId":"thread-scripted","turnId":"turn-scripted","item":{{"arguments":{{}},"contentItems":[{{"text":"Script running with cell ID cell-7","type":"inputText"}}],"id":"tool-exec","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"}},"completedAtMs":1}}}}"#).map_err(|_| ())?;
            write_turn_completed(&mut output)?;
            thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "timeout" => {
            spawn_descendant(&mode, &root)?;
            read_line(&mut input)?;
            fs::write(root.join("descendant-deadline-trigger.txt"), b"deadline").map_err(|_| ())?;
            writeln!(output, "{{\"id\":4,\"result\":{{}}}}").map_err(|_| ())?;
            output.flush().map_err(|_| ())?;
            thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "orphan" => {
            spawn_descendant(&mode, &root)?;
            write_completed(&mut output)
        }
        _ => Err(()),
    }
}

fn write_completed(output: &mut impl Write) -> Result<(), ()> {
    writeln!(output, "{}", r#"{"method":"item/completed","params":{"threadId":"thread-scripted","turnId":"turn-scripted","item":{"arguments":{"command":"apply fixture"},"contentItems":[{"text":"Script completed\nExit code: 0","type":"inputText"}],"id":"tool-apply","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"},"completedAtMs":1}}"#).map_err(|_| ())?;
    writeln!(output, "{}", r#"{"method":"item/completed","params":{"threadId":"thread-scripted","turnId":"turn-scripted","item":{"arguments":{"command":"verify fixture"},"contentItems":[{"text":"Script completed\nExit code: 0","type":"inputText"}],"id":"tool-verify","status":"completed","success":true,"tool":"exec","type":"dynamicToolCall"},"completedAtMs":2}}"#).map_err(|_| ())?;
    write_turn_completed(output)
}

fn write_turn_completed(output: &mut impl Write) -> Result<(), ()> {
    writeln!(output, "{}", r#"{"method":"turn/completed","params":{"threadId":"thread-scripted","turn":{"id":"turn-scripted","items":[{"id":"agent-final","text":"Delivery complete.","type":"agentMessage"}],"itemsView":"summary","status":"completed","error":null}}}"#).map_err(|_| ())?;
    output.flush().map_err(|_| ())
}

fn spawn_descendant(mode: &str, root: &std::path::Path) -> Result<(), ()> {
    let executable = env::current_exe().map_err(|_| ())?;
    let child = Command::new(executable)
        .arg("--descendant")
        .env(MODE, mode)
        .env(DESCENDANT_TRIGGER, root.join("descendant-trigger.txt"))
        .env(
            DESCENDANT_DEADLINE,
            root.join("descendant-deadline-trigger.txt"),
        )
        .env(DESCENDANT_EFFECT, root.join("descendant-effect.txt"))
        .spawn()
        .map_err(|_| ())?;
    fs::write(root.join("descendant.pid"), child.id().to_string()).map_err(|_| ())
}

fn run_descendant() -> Result<(), ()> {
    if required(MODE)? == "orphan" {
        thread::sleep(Duration::from_millis(800));
        return fs::write(required_path(DESCENDANT_EFFECT)?, b"survived").map_err(|_| ());
    }
    let trigger = required_path(DESCENDANT_TRIGGER)?;
    let deadline = required_path(DESCENDANT_DEADLINE)?;
    let effect = required_path(DESCENDANT_EFFECT)?;
    let stop_at = Instant::now()
        .checked_add(Duration::from_secs(30))
        .ok_or(())?;
    while !trigger.exists() && !deadline.exists() {
        if Instant::now() >= stop_at {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    if deadline.exists() {
        thread::sleep(Duration::from_millis(250));
    }
    fs::write(effect, b"survived").map_err(|_| ())
}

fn read_line(input: &mut impl BufRead) -> Result<(), ()> {
    let mut line = String::new();
    let read = input.read_line(&mut line).map_err(|_| ())?;
    if read == 0 || !line.ends_with('\n') {
        return Err(());
    }
    Ok(())
}

fn required(name: &str) -> Result<String, ()> {
    env::var(name).map_err(|_| ())
}

fn required_path(name: &str) -> Result<PathBuf, ()> {
    env::var_os(name).map(PathBuf::from).ok_or(())
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing String cannot fail")
}
