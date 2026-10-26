use std::path::Path;

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
        [command, source] if command == "import-sqlite" => {
            opencodereview_server::db::import_sqlite::run_from_env(Path::new(source), None)
        }
        [command, source, option, index] if command == "import-sqlite" && option == "--archive-index" => {
            opencodereview_server::db::import_sqlite::run_from_env(Path::new(source), Some(Path::new(index)))
        }
        _ => Err(
            "Usage: ocr-server [archive-retention [--apply] | import-sqlite <opencodereview.sqlite> [--archive-index <index.sqlite>]]"
                .into(),
        ),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
