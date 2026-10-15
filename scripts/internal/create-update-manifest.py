import argparse
import datetime as dt
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--installer", type=Path)
    args = parser.parse_args()

    version = args.tag[1:] if args.tag.startswith("v") else args.tag
    if args.installer:
        installers = [args.installer]
    else:
        roots = [Path("target/release/bundle/nsis"), Path("src-tauri/target/release/bundle/nsis")]
        installers = [path for root in roots if root.is_dir() for path in root.glob("*-setup.exe")]
    if len(installers) != 1:
        raise SystemExit(f"Expected one NSIS installer, found {len(installers)}: {installers}")
    installer = installers[0]
    signature_path = Path(f"{installer}.sig")
    if not signature_path.is_file():
        raise SystemExit(f"Updater signature not found: {signature_path}")
    signature = signature_path.read_text(encoding="utf-8").strip()
    if not signature:
        raise SystemExit("Updater signature is empty")

    manifest = {
        "version": version,
        "notes": "",
        "pub_date": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
        "platforms": {
            "windows-x86_64": {
                "signature": signature,
                "url": f"https://github.com/{args.repo}/releases/download/{args.tag}/{installer.name}",
            }
        },
    }
    Path(args.output).write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
