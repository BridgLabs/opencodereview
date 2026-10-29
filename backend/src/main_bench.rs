fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("reviewbench") => opencodereview_server::bench::run_reviewbench(),
        Some("engine-version") => {
            println!("{}", opencodereview_server::sandbox::engine_version());
            Ok(())
        }
        _ => Err("Usage: ocr-bench reviewbench|engine-version".to_string()),
    };
    if let Err(error) = result {
        eprintln!("ocr-bench: {error}");
        std::process::exit(1);
    }
}
