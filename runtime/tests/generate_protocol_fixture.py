"""Generate synthetic emitted DTO fixtures on miniforum; no Hermes/model calls."""
import hashlib
import json
import sys
import tempfile
from datetime import datetime, timedelta
from pathlib import Path

from forma_runtime.contracts import UTC, new_id, stamp
from forma_runtime.server import Application
from forma_runtime.supervisor import Config


def main():
    if sys.platform != "linux": raise SystemExit("Fixtures execute only on miniforum/Linux")
    with tempfile.TemporaryDirectory() as temporary:
        config = Config(Path(temporary) / "fixture.sqlite3", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", hashlib.sha256(b"synthetic-fixture-only").hexdigest(),
            [{"id": "synthetic-model", "name": "Synthetic contract fixture", "available": True}],
            gateway_url="http://127.0.0.1:12345/v1")
        app = Application(config); store = app.store
        fixture = {"fixtureKind": "synthetic-contract-no-Hermes-execution", "capabilities": app.capabilities(),
                   "models": {"items": config.models}}
        # Unit-only admission stub. No supervisor, import, generation or real
        # gateway is started; this cannot certify Hermes compatibility.
        config._verified = True
        def post(path, value): return store.post(path, new_id(), value)[1]
        wid = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
        budgets = {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 4096, "maxDurationSeconds": 180}
        run = post("/v1/runs", {"workspaceId": wid, "message": "Synthetic fixture", "modelId": "synthetic-model", "grantRefs": [],
                               "interfaceIds": [], "budgets": budgets})
        fixture["runAccepted"] = run; store.start_next()
        spec = {"schemaVersion": 1, "kind": "components", "root": {"id": "root", "type": "stack", "children": [
            {"id": "note", "type": "text", "text": "Synthetic DTO fixture, not live account data."}]}, "datasets": []}
        proposal = store.tool(run["id"], "forma_interface_propose", {"expectedRevision": 0, "title": "Synthetic interface", "spec": spec})
        fixture["proposalDetail"] = store.get_route("/v1/interfaces/proposals/" + proposal["id"], {})
        fixture["proposalsPage"] = store.get_route("/v1/interfaces/proposals", {"workspaceId": wid})
        store.finish(run["id"], result={"completed": True, "messages": [{"role": "assistant", "content": "Synthetic final"}], "final_response": "Synthetic final"})
        interface = post("/v1/interfaces/publish", {"proposalId": proposal["id"], "expectedRevision": 0})
        fixture["interfaceDetail"] = interface
        fixture["interfacesPage"] = store.get_route("/v1/interfaces", {"workspaceId": wid})
        fixture["revisionsPage"] = store.get_route(f'/v1/interfaces/{interface["id"]}/revisions', {})
        fixture["revisionDetail"] = store.get_route(f'/v1/interfaces/{interface["id"]}/revisions/1', {})
        fixture["runPoll"] = store.get_route("/v1/runs/" + run["id"], {})
        fixture["runsPage"] = store.get_route("/v1/runs", {"workspaceId": wid})
        schedule = post("/v1/schedules", {"workspaceId": wid, "interfaceId": interface["id"], "expectedInterfaceRevision": 1,
            "prompt": "Synthetic paused request", "modelId": "synthetic-model", "cron": "0 9 * * *", "timezone": "UTC",
            "endAt": stamp(datetime.now(UTC) + timedelta(days=1)), "maxRuns": 2, "budgets": budgets, "grantRefs": []})
        fixture["scheduleDetail"] = schedule
        fixture["schedulesPage"] = store.get_route("/v1/schedules", {"workspaceId": wid})
        fixture["cancelKnown"] = post(f'/v1/runs/{run["id"]}/cancel', {})
        fixture["cancelMissing"] = post(f'/v1/runs/{new_id()}/cancel', {})
        print(json.dumps(fixture, ensure_ascii=False, indent=2))
        app.close()


if __name__ == "__main__": main()
