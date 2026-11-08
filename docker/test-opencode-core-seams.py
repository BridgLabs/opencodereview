"""Static regression checks for the Go core's cross-module seams. No runner execution."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent


def main():
    archive = (ROOT / "backend/src/review_archive/mod.rs").read_text()
    declarations = re.findall(r"^([^\n]*fn open_archive\([^\n]+)", archive, re.M)
    assert len(declarations) == 2, declarations
    assert all(line.startswith("pub(crate) fn open_archive(") for line in declarations)
    assert "#[cfg(unix)]\npub(crate) fn open_archive(" in archive
    assert "#[cfg(not(unix))]\npub(crate) fn open_archive(" in archive

    engines = (ROOT / "backend/src/providers/engines.rs").read_text()
    assert "pub(crate) fn resolve_model(" in engines
    public = engines.split("pub(crate) fn resolve(", 1)[1].split("pub(crate) fn resolve_model(", 1)[0]
    assert "PROVIDERS.iter().any" in public
    registry = re.search(r"pub\(crate\) const PROVIDERS:[^;]+;", engines)[0]
    assert re.findall(r'\("([^"]+)",', registry) == ["claude", "codex", "opencode"]

    runner = (ROOT / "backend/src/sandbox/runner.rs").read_text()
    selection = runner.index('let (model, effort) = if provider == "opencode"')
    assert re.search(r'\.ok_or\("OpenCode Go frozen catalogue is required"\)\?\s*\.validate\(&model, &effort\)\?', runner)
    assert 'crate::providers::engines::resolve_model(&provider, &model, &effort)?' in runner
    assert runner.index("let Request::Run {") < selection < runner.index(
        'let archive = archive.ok_or("Frozen archive must be installed before a stage")?;'
    )
    assert "const OPENCODE" not in engines
    assert not (ROOT / "docker/opencode-guard/models.json").exists()
    assert 'go_snapshot: Option<crate::providers::go_catalogue::Snapshot>' in runner
    print("PASS static seams: archive openers; independently validated dynamic Go snapshot; no static Go inventory")


if __name__ == "__main__":
    main()
