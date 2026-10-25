fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = match args.as_slice() {
        [] => opencodereview_server::server::run_from_env(),
        [command] if command == "archive-retention" => {
            opencodereview_server::review_archive::purge::run_from_env(false)
        }
        [command, option] if command == "archive-retention" && option == "--apply" => {
            opencodereview_server::review_archive::purge::run_from_env(true)
        }
        _ => Err("Usage: ocr-server [archive-retention [--apply]]".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
