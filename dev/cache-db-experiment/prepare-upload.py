#!/usr/bin/env python3
"""Prepare local, prebuilt staging uploads. Does not deploy or read credentials."""
import json
import pathlib
import shutil
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[2]
destination = pathlib.Path(sys.argv[1]).resolve()
binary_directory = pathlib.Path(sys.argv[2]).resolve()
revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
for service, binary in [("api", "api"), ("worker", "worker"), ("cache-service", "scope-cache-service")]:
    upload = destination / service
    (upload / "bin").mkdir(parents=True, exist_ok=True)
    (upload / service).mkdir(exist_ok=True)
    shutil.copy2(binary_directory / binary, upload / "bin" / binary)
    shutil.copy2(root / "deploy/railway/start-prebuilt.sh", upload / "start.sh")
    shutil.copy2(root / "LICENSE", upload / "LICENSE")
    shutil.copy2(root / "NOTICE", upload / "NOTICE")
    config = json.loads((root / "deploy/railway/prebuilt-backend-git.railpack.json").read_text())
    config["deploy"]["inputs"][0]["include"] += ["LICENSE", "NOTICE"]
    for file in [upload / "railpack.json", upload / service / "railpack.json"]:
        file.write_text(json.dumps(config))
    (upload / ".scope-deployment-sha").write_text(revision + "\n")
    (upload / "railway.json").write_text(json.dumps({
        "$schema": "https://railway.com/railway.schema.json",
        "build": {"builder": "RAILPACK", "buildCommand": "true", "watchPatterns": []},
        "deploy": {"startCommand": "sh ./start.sh", "healthcheckPath": "/readyz" if service == "cache-service" else "/healthz", "healthcheckTimeout": 60},
    }))
    print(upload)
