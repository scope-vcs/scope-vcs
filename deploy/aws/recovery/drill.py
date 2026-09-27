"""Run a verified recovery set as a sealed Scope stack, prove it with canaries, and record evidence.

Every container shares one network namespace with no interface except loopback, so restored
data and background jobs cannot reach production or any other external service. Containers only
receive the environment written here; nothing is inherited from the operator's shell.
"""

import argparse
import datetime
import json
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from common import Incomplete, digest, keys

# Agreed in README.md "Drill targets"; change both together.
RECOVERY_TIME_TARGET = datetime.timedelta(hours=4)
BACKUP_AGE_TARGET = datetime.timedelta(hours=26)

POSTGRES_IMAGE = "postgres:18.6@sha256:86c951e05bf56c93d95d397747fb8820ac76cc3bedb78f43abd83eedbe3666ae"
S3_IMAGE = "versity/versitygw:v1.8.0@sha256:30292fc2eeacc67a36993b01f7a7a5e3361a19cced0e80c1d71cfa2a4b0a2499"
TOOLS_IMAGE = "scope-recovery-drill-tools"
SERVICES_IMAGE = "scope-recovery-drill-services"
PREFIX = "scope-recovery-drill"
LABEL = "scope.recovery-drill=1"
DATABASE = "scope_recovery_drill_" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d")
BINARIES = {"api": "api", "scope-maintenance": "scope-maintenance", "worker": "scope-worker",
            "scope-media-service": "scope-media-service"}
API = "http://127.0.0.1:8080"
MEDIA = "http://127.0.0.1:8083"
S3 = "http://127.0.0.1:7070"
APP_ORIGIN = "http://127.0.0.1:3000"  # Nothing listens; the web canary is not part of this drill.
HERE = Path(__file__).resolve().parent
REPOSITORY_ROOT = HERE.parents[2]


def now():
    return datetime.datetime.now(datetime.timezone.utc)


def docker(*args, env=None):
    result = subprocess.run(["docker", *args], env=env, capture_output=True, text=True, check=False)
    if result.returncode:
        # Diagnostics may quote restored data, so they go to the operator's terminal only;
        # the evidence records a fixed message.
        print(result.stderr.strip()[-2000:], file=sys.stderr)
        raise Incomplete(f"docker {args[0]} failed")
    return result.stdout.strip()


def run(name, image, env, *, detach=True, mounts=(), command=(), network=f"container:{PREFIX}-db"):
    # `--env NAME` copies the value from the Docker client's environment, which keeps secrets
    # out of process arguments and carries multi-line PEM keys intact.
    options = ["--detach", "--name", f"{PREFIX}-{name}"] if detach else ["--rm"]
    variables = [argument for variable in env for argument in ("--env", variable)]
    volumes = [argument for mount in mounts for argument in ("--volume", mount)]
    return docker("run", *options, "--label", LABEL, "--network", network, *variables, *volumes, image, *command,
                  env={**os.environ, **env})


def build_images(binaries):
    git = json.loads((REPOSITORY_ROOT / "dev/tool-versions.json").read_text())["git"]
    with tempfile.TemporaryDirectory(prefix="scope-drill-image-") as context:
        context = Path(context)
        (context / "bin").mkdir()
        for source, target in BINARIES.items():
            shutil.copy2(Path(binaries) / source, context / "bin" / target)
        shutil.copy2(REPOSITORY_ROOT / "deploy/railway/install-git.sh", context)
        shutil.copytree(REPOSITORY_ROOT / "dependency-analyzer", context / "dependency-analyzer",
                        ignore=shutil.ignore_patterns("node_modules"))
        shutil.copy2(REPOSITORY_ROOT / "legal/third-party-dependency-analyzer.txt", context / "dependency-analyzer")
        # The release worker image is a superset of the API and media images: reviewed Git,
        # Node for the dependency analyzer, and the non-root runtime user.
        docker("build", "--quiet", "--file", str(REPOSITORY_ROOT / "deploy/railway/worker.Dockerfile"),
               "--build-arg", f"GIT_VERSION={git['version']}", "--build-arg", f"GIT_SOURCE_SHA256={git['sourceSha256']}",
               "--tag", SERVICES_IMAGE, str(context))
    docker("build", "--quiet", "--file", str(HERE / "drill.Dockerfile"), "--tag", TOOLS_IMAGE, str(HERE))


def signing_key():
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    key = Ed25519PrivateKey.generate()
    private = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
    public = key.public_key().public_bytes(serialization.Encoding.PEM, serialization.PublicFormat.SubjectPublicKeyInfo)
    return private.decode(), public.decode()


def bucket_env(prefix, name, credentials):
    return {f"{prefix}_ENDPOINT": S3, f"{prefix}_NAME": name, f"{prefix}_REGION": "us-east-1",
            f"{prefix}_ACCESS_KEY_ID": credentials[0], f"{prefix}_SECRET_ACCESS_KEY": credentials[1],
            f"{prefix}_FORCE_PATH_STYLE": "true"}


def tools(script, env, restored):
    mounts = [f"{HERE}:/recovery:ro", f"{restored}:/restored:ro"]
    return run("tools", TOOLS_IMAGE, env, detach=False, mounts=mounts,
               command=["python3", "-c", f"import sys; sys.path.insert(0, '/recovery')\n{script}"])


def up(restored, repo):
    escrow = keys(Path(restored) / "keys.json")
    password, credentials = secrets.token_hex(16), ("drill", secrets.token_hex(16))
    database_url = f"postgres://scope:{password}@127.0.0.1:5432/{DATABASE}"
    run("db", POSTGRES_IMAGE, {"POSTGRES_USER": "scope", "POSTGRES_PASSWORD": password, "POSTGRES_DB": DATABASE},
        network="none")
    interfaces = {line.split(":")[0].strip() for line in docker("exec", f"{PREFIX}-db", "cat", "/proc/net/dev").splitlines()[2:]}
    if interfaces != {"lo"}:
        raise Incomplete("the drill namespace has a network interface besides loopback")
    run("s3", S3_IMAGE, {}, mounts=["/data"],
        command=["--access", credentials[0], "--secret", credentials[1], "--port", "127.0.0.1:7070", "posix", "/data"])
    tools("from drill_stage import wait_for_database; wait_for_database()", {"SCOPE_RECOVERY_DRILL_DATABASE_URL": database_url}, restored)
    users = [{"role": role, "handle": f"recovery-drill-{role}", "token": "scope_cli_" + secrets.token_hex(32)}
             for role in ("member", "outsider")]
    stage = json.loads(tools("import drill_stage; drill_stage.main()", {
        "SCOPE_DRILL_RESTORED": "/restored", "SCOPE_RECOVERY_DRILL_DATABASE_URL": database_url,
        "SCOPE_DRILL_S3_ENDPOINT": S3, "SCOPE_DRILL_S3_ACCESS_KEY": credentials[0],
        "SCOPE_DRILL_S3_SECRET_KEY": credentials[1], "SCOPE_DRILL_REPO": repo,
        "SCOPE_DRILL_USERS": json.dumps(users)}, restored))
    run("verify", SERVICES_IMAGE, {"DATABASE_URL": database_url}, detach=False,
        command=["/app/bin/scope-maintenance", "verify"])
    media_private, media_public = signing_key()
    objects = {"DATABASE_URL": database_url, "SCOPE_OBJECT_ENCRYPTION_KEY": escrow["SCOPE_OBJECT_ENCRYPTION_KEY"],
               **bucket_env("SCOPE_BUCKET", "scope-drill-objects", credentials)}
    run("api", SERVICES_IMAGE, {**objects, "PORT": "8080", "SCOPE_GIT_PUBLIC_URL": API, "SCOPE_APP_ORIGIN": APP_ORIGIN,
                                "SCOPE_MEDIA_PUBLIC_URL": MEDIA, "SCOPE_MEDIA_GRANT_PRIVATE_KEY": media_private,
                                # No cache service runs; the API only needs well-formed settings to start.
                                "SCOPE_CACHE_URL": "http://127.0.0.1:8090", "SCOPE_CACHE_BACKEND": "recovery-drill",
                                "SCOPE_CACHE_GRANT_PRIVATE_KEY": signing_key()[0]},
        command=["/app/bin/api"])
    run("worker", SERVICES_IMAGE, {**objects, "PORT": "8081", "SCOPE_WORKER_ID": "recovery-drill"})
    run("media", SERVICES_IMAGE, {"DATABASE_URL": database_url, "PORT": "8083",
                                  "SCOPE_MEDIA_ENCRYPTION_KEY": escrow["SCOPE_MEDIA_ENCRYPTION_KEY"],
                                  "SCOPE_MEDIA_GRANT_PUBLIC_KEY": media_public, "SCOPE_MEDIA_ALLOWED_ORIGIN": APP_ORIGIN,
                                  **bucket_env("SCOPE_MEDIA_BUCKET", "scope-drill-media", credentials)},
        command=["/app/bin/scope-media-service"])
    tools("from drill_stage import wait_for_services; wait_for_services()", {}, restored)
    return stage, {user["role"]: user["token"] for user in users}, database_url


GIT_CANARY = r"""
set -eu
mkdir -p "$HOME" && cd "$HOME"
session_dir="$HOME/.config/scope/sessions"
mkdir -p "$session_dir"
printf %s "$MEMBER_TOKEN" > "$session_dir/cli-session-$(printf %s "$API" | od -An -tx1 | tr -d ' \n')"
chmod 600 "$session_dir"/*
git clone --quiet "$API/git/public/$REPO" public
if test -e "public/$PRIVATE_PATH"; then echo "anonymous clone contains the private path" >&2; exit 1; fi
/drill/scope --api-url "$API" clone "$REPO" member >/dev/null
test -f "member/$PRIVATE_PATH" || { echo "member clone lacks the private path" >&2; exit 1; }
cd member
# Scope rejects pushes that leave the tree unchanged, so the canary adds a file.
mkdir -p recovery-drill && date -u +%Y-%m-%dT%H:%M:%SZ > recovery-drill/canary.txt
git add recovery-drill/canary.txt
git -c user.name="Recovery drill" -c user.email=member@recovery-drill.invalid commit --quiet -m "Recovery drill write canary"
pushed="$(git rev-parse HEAD)"
/drill/scope --api-url "$API" push --main --no-review >/dev/null
cd "$HOME"
/drill/scope --api-url "$API" clone "$REPO" fetched >/dev/null
test "$(git -C fetched rev-parse HEAD)" = "$pushed" || { echo "fresh clone does not contain the pushed commit" >&2; exit 1; }
"""


def canaries(restored, cli, repo, private_path, tokens, database_url):
    results = {}
    try:
        run("git-canary", SERVICES_IMAGE, {"API": API, "REPO": repo, "PRIVATE_PATH": private_path,
                                           "MEMBER_TOKEN": tokens["member"], "HOME": "/tmp/drill-home",
                                           # `scope clone` installs `!scope git-credential` as the helper.
                                           "PATH": "/drill:/opt/git/bin:/usr/local/bin:/usr/bin:/bin"},
            detach=False, mounts=[f"{Path(cli).resolve()}:/drill/scope:ro"], command=["sh", "-c", GIT_CANARY])
        results["git"] = "passed: anonymous public clone, member private clone, push to main, fresh clone"
    except Incomplete as error:
        results["git"] = f"failed: {error}"
    output = tools("import drill_stage; drill_stage.http_canaries()", {
        "SCOPE_DRILL_API": API, "SCOPE_DRILL_REPO": repo, "SCOPE_DRILL_PRIVATE_PATH": private_path,
        "SCOPE_DRILL_MEMBER_TOKEN": tokens["member"], "SCOPE_DRILL_OUTSIDER_TOKEN": tokens["outsider"],
        "SCOPE_RECOVERY_DRILL_DATABASE_URL": database_url}, restored)
    results.update(json.loads(output))
    return results


def down():
    names = docker("ps", "--all", "--quiet", "--filter", f"label={LABEL}").split()
    if names:
        docker("rm", "--force", "--volumes", *names)
    return len(names)


def hours(delta):
    return round(delta.total_seconds() / 3600, 2)


def drill(args):
    restore_started = datetime.datetime.fromisoformat(args.restore_started_at)
    if restore_started.tzinfo is None:
        raise SystemExit("--restore-started-at needs a UTC offset, for example 2026-09-27T14:05:00+00:00")
    manifest = json.loads((Path(args.restored) / "manifest.json").read_bytes())
    captured = datetime.datetime.fromisoformat(manifest["database"]["captured_at"])
    evidence = {"revision": args.revision, "captured_at": captured.isoformat(),
                "restore_started_at": restore_started.isoformat(),
                "binaries_sha256": {name: digest(Path(args.binaries) / name) for name in BINARIES},
                "cli_sha256": digest(args.cli), "targets_hours": {
                    "recovery_time": hours(RECOVERY_TIME_TARGET), "backup_age": hours(BACKUP_AGE_TARGET)}}
    try:
        if docker("ps", "--all", "--quiet", "--filter", f"label={LABEL}"):
            raise Incomplete("a drill stack already exists; inspect it, then run `drill.py down`")
        build_images(args.binaries)
        evidence["stage"], tokens, database_url = up(args.restored, args.repo)
        evidence["services_ready_at"] = now().isoformat()
        evidence["canaries"] = canaries(args.restored, args.cli, args.repo, args.private_path, tokens, database_url)
    except Incomplete as error:
        evidence["failure"] = str(error)
    finished = now()
    evidence["finished_at"] = finished.isoformat()
    evidence["recovery_time_hours"] = hours(finished - restore_started)
    evidence["backup_age_hours"] = hours(restore_started - captured)
    evidence["complete"] = ("failure" not in evidence
                            and all(result.startswith("passed") for result in evidence["canaries"].values())
                            and finished - restore_started <= RECOVERY_TIME_TARGET
                            and restore_started - captured <= BACKUP_AGE_TARGET)
    if not args.keep:
        evidence["removed_containers"] = down()
    Path(args.evidence).write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    print(json.dumps(evidence, indent=2, sort_keys=True))
    return 0 if evidence["complete"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    start = commands.add_parser("run", help="restore, start, prove, and record one drill")
    start.add_argument("--restored", required=True, help="destination verified by restore.py")
    start.add_argument("--binaries", required=True, help="release api, scope-maintenance, worker, scope-media-service")
    start.add_argument("--cli", required=True, help="scope CLI binary for the Git canary")
    start.add_argument("--revision", required=True, help="release revision the binaries were built from")
    start.add_argument("--repo", required=True, help="restored owner/name with at least one private path")
    start.add_argument("--private-path", required=True, help="a private file in --repo")
    start.add_argument("--restore-started-at", required=True, help="ISO time with offset when the archive download began")
    start.add_argument("--evidence", required=True, help="where to write the evidence JSON")
    start.add_argument("--keep", action="store_true", help="leave the stack running for inspection")
    commands.add_parser("down", help="remove every drill container and its data")
    args = parser.parse_args()
    os.umask(0o077)
    if args.command == "down":
        print(f"removed {down()} drill containers")
        return 0
    return drill(args)


if __name__ == "__main__":
    sys.exit(main())
