use freefire_online::{check_online, check_online_blocking};
use serde_json::json;
use std::env;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: ffonline <player_id>");
        std::process::exit(2);
    }
    let id = &args[1];
    match check_online(id).await {
        Ok(res) => {
            let out = json!({
                "player_id": res.player_id,
                "profile": res.profile,
                "online": res.online,
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// keep blocking entrypoint available for non-async callers
#[allow(dead_code)]
fn _blocking(id: &str) {
    match check_online_blocking(id) {
        Ok(res) => println!("{}", serde_json::to_string_pretty(&serde_json::json!({"player_id":res.player_id,"profile":res.profile,"online":res.online})).unwrap()),
        Err(e) => eprintln!("error: {e}"),
    }
}