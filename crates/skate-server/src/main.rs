use skate_server::{Host, Options, USAGE};
use std::{
    io::{BufRead, Write},
    time::{Duration, Instant},
};

fn run() -> Result<i32, String> {
    let args:Vec<_>=std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg|arg=="--validate-resources") {
        if args.len()!=2 {return Err("--validate-resources requires exactly one configuration path".into());}
        println!("{}",skate_server::resources::validate_configuration(std::path::Path::new(&args[1]))?);
        return Ok(0);
    }
    let Some(options) = Options::parse(args)? else {
        println!("{USAGE}");
        return Ok(0);
    };
    let session = options.session;
    let max_players = options.max_players;
    let mut host = Host::bind(options)?;
    let map = host.map_fingerprint();
    println!(
        "Dedicated server listening on {} | session={session} map={map:016x} players=0/{max_players}",
        host.local_addr().map_err(|e| e.to_string())?
    );
    println!(
        "Client-predicted skating; server-authorized collisions, shoves and resource state. Type quit for clean shutdown."
    );
    if let Some(address) = host.account_address() {
        println!("Account login and administration HTTPS listening on {address}");
    }
    if let Some(address) = host.resource_address() {
        println!(
            "Resource HTTP listening on {address}; console: resources, start/stop/restart/ensure ID, command NAME, quit"
        );
    }
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let (send, receive) = std::sync::mpsc::sync_channel::<String>(16);
    std::thread::spawn(move || {
        // Read bounded lines without allowing an untrusted pipe to allocate an
        // arbitrarily large String. The bounded queue backpressures producers.
        let mut input = std::io::stdin().lock();
        let mut line = Vec::new();
        loop {
            let buffer = match input.fill_buf() {
                Ok(v) => v,
                Err(_) => break,
            };
            if buffer.is_empty() {
                break;
            }
            let length = buffer
                .iter()
                .position(|b| *b == b'\n')
                .map_or(buffer.len(), |n| n + 1);
            for &byte in &buffer[..length] {
                if byte == b'\n' {
                    if line.len() <= 4096 {
                        if let Ok(value) = String::from_utf8(std::mem::take(&mut line)) {
                            if send.send(value).is_err() {
                                return;
                            }
                        }
                    }
                    line.clear();
                } else if line.len() <= 4096 {
                    line.push(byte);
                }
            }
            input.consume(length);
        }
    });
    let supervised=std::env::var_os("SKATE_SUPERVISOR_CONTROL").is_some();
    let mut heartbeat=Instant::now()-Duration::from_secs(1);
    let mut players = 0;
    let mut last_status = Instant::now();
    loop {
        let before = Instant::now();
        for _ in 0..8 {
            let Ok(line) = receive.try_recv() else { break };
            if supervised && line.trim() == "supervisor-stop" {host.shutdown();return Ok(2);}
            if line.trim() == "quit" {
                if supervised {skate_server::supervision::shutdown_intent()?;}
                host.shutdown();
                return Ok(0);
            }
            if line.trim().is_empty() {
                continue;
            }
            match host.resource_command(&line) {
                Ok(result) => println!("{result}"),
                Err(error) => eprintln!("Console: {error}"),
            }
        }
        host.step().map_err(|e| format!("UDP service: {e}"))?;
        if supervised && heartbeat.elapsed()>=Duration::from_millis(250) {
            skate_server::supervision::heartbeat()?;
            heartbeat=Instant::now();
        }
        if let Some(restart)=host.exit_requested() {
            host.shutdown();
            return Ok(if restart {75}else{0});
        }
        let current = host.player_count();
        if current != players || last_status.elapsed() >= Duration::from_secs(30) {
            println!("Players: {current}/{max_players}");
            players = current;
            last_status = Instant::now();
        }
        // Fixed service cadence; never let a packet flood starve timers/effects.
        std::thread::sleep(Duration::from_millis(10).saturating_sub(before.elapsed()));
    }
}

fn main() {
    match run() {
        Ok(code)=>std::process::exit(code),
        Err(error)=>{eprintln!("skate-server: {error}\n{USAGE}");std::process::exit(2);}
    }
}
