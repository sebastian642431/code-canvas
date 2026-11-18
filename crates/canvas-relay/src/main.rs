use std::process::ExitCode;
use std::time::Duration;

use canvas_relay::{Limits, Relay};

const USAGE: &str = "\
canvas-relay: pairs the two ends of a shared Code Canvas session and passes
their sealed bytes along. It never has the secret of an invitation.

Usage: canvas-relay --listen <address:port> [options]

  --listen <address:port>    where to listen, such as 0.0.0.0:7878
  --rooms <n>                rooms open at once (256)
  --connections <n>          connections open at once (1024)
  --per-address <n>          connections from one address (64)
  --rooms-per-address <n>    rooms opened by one address (8)
  --guests <n>               guests through one room (32)
  --idle <seconds>           a pair with nothing crossing is parted (300)
  --room-hours <hours>       a room ends this long after it opened (24)
  --largest-mb <MiB>         the largest frame passed on (16)
  --quiet                    say nothing of rooms opening and ending
  --version                  the version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut limits = Limits::default();
    let (mut listen, mut quiet) = (None, false);
    let mut at = 0;
    while at < args.len() {
        let flag = args[at].as_str();
        let value = args.get(at + 1).map(String::as_str);
        let number = || value.and_then(|value| value.parse::<u64>().ok()).filter(|n| *n > 0);
        let mut took = true;
        let fine = match flag {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("canvas-relay {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--quiet" => {
                quiet = true;
                took = false;
                true
            }
            "--listen" => {
                listen = value.map(str::to_string);
                listen.is_some()
            }
            "--rooms" => number().map(|n| limits.rooms = n as usize).is_some(),
            "--connections" => number().map(|n| limits.connections = n as usize).is_some(),
            "--per-address" => number().map(|n| limits.per_address = n as usize).is_some(),
            "--rooms-per-address" => number().map(|n| limits.rooms_per_address = n as usize).is_some(),
            "--guests" => number().map(|n| limits.guests = n as usize).is_some(),
            "--idle" => number().map(|n| limits.idle = Duration::from_secs(n)).is_some(),
            "--room-hours" => number().map(|n| limits.room_life = Duration::from_secs(n * 3600)).is_some(),
            "--largest-mb" => number().filter(|n| *n <= 1024).map(|n| limits.largest = (n as usize) << 20).is_some(),
            _ => false,
        };
        if !fine {
            eprint!("canvas-relay: {flag} is not understood here\n\n{USAGE}");
            return ExitCode::from(2);
        }
        at += if took { 2 } else { 1 };
    }
    let Some(listen) = listen else {
        eprint!("canvas-relay: say where to listen with --listen\n\n{USAGE}");
        return ExitCode::from(2);
    };
    match Relay::start(listen.as_str(), limits, quiet) {
        Ok(relay) => {
            eprintln!("canvas-relay: listening on {}", relay.address());
            relay.run();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("canvas-relay: could not listen on {listen}: {e}");
            ExitCode::FAILURE
        }
    }
}
