use self::markdown::Markdown;
use clap::Parser;
use log::LevelFilter;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::Duration,
};
use sysinfo::{CpuExt, PidExt, ProcessExt, System, SystemExt};
use self::report::{Metrics, Report};

mod markdown;
mod report;

#[derive(Debug, Parser)]
#[clap(version)]
struct Args {
    /// Path to workspace directory.
    #[clap(short)]
    workspace_dir: PathBuf,

    /// Path to output file.
    #[clap(short)]
    output_dir: PathBuf,

    /// Connection count of each benchmark.
    #[clap(short, default_value = "500")]
    connections: usize,

    /// Duration of each benchmark in seconds.
    #[clap(short, default_value = "30")]
    duration: usize,

    /// Url for each benchmark.
    #[clap(short, default_value = "http://127.0.0.1:3000")]
    url: String,

    /// Cooling down for each benchmark.
    #[clap(long, default_value = "5")]
    cd: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Cargo {
    workspace: Workspace,
}

#[derive(Debug, Serialize, Deserialize)]
struct Workspace {
    members: Vec<PathBuf>,
}

fn main() {
    let args = Args::parse();

    env_logger::builder()
        .format(|buf, record| {
            let ts = buf.timestamp();
            let level = buf.default_styled_level(record.level());
            writeln!(buf, "[{} {}] {}", ts, level, record.args())
        })
        .filter_module("bench_bot", LevelFilter::Info)
        .init();

    log::info!("Bench Bot started.");

    let ws_toml_path = args.workspace_dir.join("Cargo.toml");
    let ws_toml = fs::read(&ws_toml_path).unwrap();

    let cargo: Cargo = toml::from_slice(&ws_toml).unwrap();
    let members = expand_members(cargo.workspace.members, &args.workspace_dir);

    let mut exclude = Vec::new();

    for member in &members {
        log::info!("Building {:?}", member);

        // go build -o my_go_app
        let member_str = member.to_string_lossy();
        let output = if member_str.starts_with("go_") {
            // If the member starts with "go_", use "go build"
            Command::new("go")
                .args(&["build"])
                .current_dir(args.workspace_dir.join(&member))
                .output()
                .expect("Failed to execute Go build")
        } else {
            // Default case: use "cargo build --release"
            Command::new("cargo")
                .args(&["build", "--release"])
                .current_dir(args.workspace_dir.join(&member))
                .output()
                .expect("Failed to execute Cargo build")
        };

        if !output.status.success() {
            log::error!(
                "Building {:?} failed: \n{}",
                member,
                String::from_utf8_lossy(&output.stderr)
            );
            exclude.push(member.clone());
        }
    }

    let sys = System::new_all();

    let cpu_name = sys.global_cpu_info().brand();
    let cpu_count = (sys.cpus().len() - 1).to_string();
    let conn_count = args.connections.to_string();
    let duration = format!("{}s", args.duration);
    let cd = args.cd;
    let members_len = members.len();

    let rewrk_args = [
        "-t",
        &cpu_count,
        "-c",
        &conn_count,
        "-d",
        &duration,
        "-h",
        &args.url,
    ];

    let mut bench_command = "rewrk".to_owned();
    for arg in rewrk_args {
        bench_command.push(' ');
        bench_command.push_str(arg);
    }

    let mut base_md = Markdown::new();

    base_md.add_item("Generated by bench-bot.");
    base_md.add_item("# Hardware");
    base_md.add_item("## Cpu");
    base_md.add_item(cpu_name);
    base_md.add_item("# Benchmark");
    base_md.add_item("Command:");
    base_md.add_item(format!("```\n{}\n```", bench_command));

    let mut output_map = HashMap::new();
    let mut reports = Vec::with_capacity(members.len());

    for (index, member) in members.iter().enumerate() {
        if exclude.contains(member) {
            log::warn!("Skipping {:?} because build was failed.", member);
        } else {
            let bench_type = member
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();

            let framework_name = member.file_name().unwrap().to_str().unwrap();

            let result_md = output_map.entry(bench_type).or_insert(Markdown::new());

            log::info!("Benchmarking {:?}", member);

            let member_str = member.to_string_lossy();

            println!("{:?}", args.workspace_dir.join(&member));
            let mut server = if member_str.starts_with("go_") {
                // If the member starts with "go_", use "go run"
                Command::new("go")
                    .args(&["run", "."])  // `.` indicates the current directory for Go run
                    .current_dir(args.workspace_dir.join(&member))
                    .spawn()
                    .expect("Failed to execute Go run")
            } else {
                // Default case: use "cargo run --release -q"
                Command::new("cargo")
                    .args(&["run", "--release", "-q"])
                    .current_dir(args.workspace_dir.join(&member))
                    .spawn()
                    .expect("Failed to execute Cargo run")
            };

            thread::sleep(Duration::from_secs(1));

            let pid = PidExt::from_u32(server.id());
            let (tx, rx) = mpsc::channel::<()>();

            let mem_usage_thread = thread::spawn(move || {
                let mut sys = System::new();
                let mut max_memory = 0;
                while rx.try_recv().is_err() {
                    sys.refresh_process(pid);
                    max_memory =
                        max_memory.max(sys.process(pid).map(ProcessExt::memory).unwrap_or(0));

                    thread::sleep(Duration::from_millis(100));
                }
                max_memory
            });

            let output = Command::new("rewrk").args(rewrk_args).output().unwrap();

            tx.send(()).unwrap();
            let _ = server.kill();
            let max_memory = mem_usage_thread.join().unwrap();
            let max_memory =
                f64::from(u32::try_from(max_memory).expect("mem usage too high")) / 1024.0;

            if output.stderr.len() > 0 {
                log::error!(
                    "Benchmarking {:?} failed: \n{}",
                    member,
                    String::from_utf8_lossy(&output.stderr)
                );
            } else {
                let stdout = String::from_utf8_lossy(&output.stdout);

                result_md.add_item(format!("## {}", framework_name));
                result_md.add_item(format!("Maximum Memory Usage: {:.1} MB", max_memory));
                result_md.add_item(format!("```\n{}\n```", stdout.trim()));

                if let Ok(metrics) = stdout.parse::<Metrics>() {
                    reports.push(Report::new(
                        framework_name,
                        max_memory,
                        metrics,
                    ));
                } else {
                    log::warn!("Could not parse benchmark result: {}", stdout);
                }
            }

            // lets CPU cooling down, ignore last member.
            if index != members_len - 1 {
                thread::sleep(Duration::from_secs(cd));
            }
        }
    }

    for (bench_type, result_md) in output_map {
        let mut output_md = base_md.clone();

        output_md.add_item("## Comparisons");
        output_md.add_item(Report::generate_from(&reports));

        output_md.add_item(result_md.finish());

        let output_path = args.output_dir.join(format!("{}.md", bench_type));

        log::info!("Writing output to {:?}.", output_path);
        fs::write(output_path, output_md.finish()).unwrap();
    }
}

fn expand_members(members: Vec<PathBuf>, ws_dir: &Path) -> Vec<PathBuf> {
    let mut new_members = Vec::new();
    for member in members {
        if member.components().last() == Some(Component::Normal(&OsStr::new("*"))) {
            let parent_dir = member.parent().unwrap();
            for entry in fs::read_dir(ws_dir.join(parent_dir)).unwrap() {
                let entry = entry.unwrap();
                if entry.metadata().unwrap().is_dir() {
                    new_members.push(parent_dir.join(entry.path().file_name().unwrap()));
                }
            }
        } else {
            new_members.push(member);
        }
    }
    new_members.sort();
    new_members
}
