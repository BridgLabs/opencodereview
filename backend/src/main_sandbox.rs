fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("engine-version") => {
            println!("{}", opencodereview_server::sandbox::engine_version());
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
        Some("opencode-tool") => std::process::exit(opencodereview_server::sandbox::serve_opencode_tool(
            std::io::stdin().lock(),
        )),
        Some("egress") => opencodereview_server::sandbox::serve_egress(),
        _ => Err("Usage: ocr-sandbox runner|egress|engine-version|tool-guard|opencode-tool".to_string()),
    };
    if let Err(error) = result {
        eprintln!("Sandbox stopped: {error}");
        std::process::exit(1);
    }
}
