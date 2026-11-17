fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("engine-version") => {
            println!("{}", opencodereview_server::sandbox::engine_version());
            Ok(())
        }
        // Cursor's hooks read a JSON decision; exit status 2 denies as well.
        Some("tool-guard") if std::env::args().nth(2).as_deref() == Some("cursor") => {
            match opencodereview_server::sandbox::guard_cursor_tool(std::io::stdin().lock()) {
                Ok(()) => println!("{}", serde_json::json!({ "permission": "allow" })),
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::json!({ "permission": "deny", "user_message": error, "agent_message": error })
                    );
                    std::process::exit(2);
                }
            }
            Ok(())
        }
        Some("tool-guard") => {
            if let Err(error) = opencodereview_server::sandbox::guard_tool(std::io::stdin().lock()) {
                eprintln!("{error}");
                std::process::exit(2);
            }
            Ok(())
        }
        Some("runner") => opencodereview_server::sandbox::serve_runner(),
        Some("egress") => opencodereview_server::sandbox::serve_egress(),
        _ => Err("Usage: ocr-sandbox runner|egress|engine-version|tool-guard [cursor]".to_string()),
    };
    if let Err(error) = result {
        eprintln!("Sandbox stopped: {error}");
        std::process::exit(1);
    }
}
