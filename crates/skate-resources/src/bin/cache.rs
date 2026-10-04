use skate_resources::{Cache, Error, Limits, Result, download_set};
use std::{collections::BTreeMap, net::SocketAddr, sync::atomic::AtomicBool};
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!(
            "skate-resource-cache inspect CACHE\nskate-resource-cache prune CACHE [TARGET_BYTES]\nskate-resource-cache forget CACHE SOURCE\nskate-resource-cache fetch CACHE HOST:PORT REVISION SOURCE\n\nInspect verified resource history, prune inactive content, unpin a source, or fetch without executing Lua."
        );
        return Ok(());
    }
    let root = args
        .get(1)
        .ok_or_else(|| Error("missing cache directory".into()))?;
    let cache = Cache::open(root, Limits::default())?;
    match args[0].as_str() {
        "inspect" if args.len() == 2 => println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"disk_bytes":cache.disk_bytes()?,"history":cache.inventory()?})
            )?
        ),
        "prune" if args.len() == 2 || args.len() == 3 => {
            let target = args
                .get(2)
                .map(|s| s.parse::<u64>())
                .transpose()
                .map_err(|_| Error("invalid byte target".into()))?
                .unwrap_or(0);
            println!(
                "{}",
                serde_json::json!({"removed_bytes":cache.prune(target)?,"remaining_bytes":cache.disk_bytes()?})
            );
        }
        "forget" if args.len() == 3 => {
            cache.deactivate(&args[2])?;
            println!("{}", serde_json::json!({"unpinned_source":args[2]}));
        }
        "fetch" if args.len() == 5 => {
            let endpoint: SocketAddr = args[2]
                .parse()
                .map_err(|_| Error("endpoint must be an IP address and port".into()))?;
            let report = download_set(
                endpoint,
                &args[3],
                &cache,
                &args[4],
                &AtomicBool::new(false),
            )?;
            let roots: BTreeMap<_, _> = report
                .roots
                .iter()
                .map(|(id, path)| (id, path.to_string_lossy()))
                .collect();
            println!(
                "{}",
                serde_json::json!({"revision":report.set.revision,"downloaded_bytes":report.downloaded_bytes,"reused_bytes":report.reused_bytes,"roots":roots})
            );
        }
        _ => return Err(Error("invalid arguments; use --help".into())),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("resource cache: {error}");
        std::process::exit(1);
    }
}
