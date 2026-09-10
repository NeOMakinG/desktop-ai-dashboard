"""Strict product wire validation. No Hermes imports or provider access."""
from __future__ import annotations

import json
import math
import re
import uuid
from datetime import datetime, timedelta, timezone
from urllib.parse import urlsplit

REVISION = "349e6611a1c5d846a865368dd6c386b78edd1a54"
VERSION = "forma-runtime-v1"
LIMITS = {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 4096,
          "maxDurationSeconds": 180, "maxToolResultBytes": 262144}
GOOGLE_TOOLS = ("forma_gmail_list_metadata", "forma_calendar_list_events")
TOOLS = ("forma_interface_list", "forma_interface_get", "forma_interface_propose",
         "forma_schedule_create", *GOOGLE_TOOLS)
TERMINAL = frozenset(("succeeded", "failed", "cancelled", "interrupted", "blocked"))
IDENT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$")
FORBIDDEN_IDS = frozenset(("__proto__", "prototype", "constructor"))
UTC = timezone.utc


class Fault(Exception):
    def __init__(self, code: str, message: str, status: int = 422):
        super().__init__(message)
        self.code, self.message, self.status = code, message, status

    def wire(self):
        return {"error": {"code": self.code, "message": self.message,
                          "retryable": self.status in (429, 503)}}


def require(condition, message="Invalid request", code="invalid_request", status=422):
    if not condition:
        raise Fault(code, message, status)


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def decode(raw):
    def pairs(items):
        out = {}
        for key, value in items:
            require(key not in out, "Duplicate JSON field")
            out[key] = value
        return out
    try:
        return json.loads(raw, object_pairs_hook=pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite")))
    except (ValueError, UnicodeError, RecursionError) as exc:
        raise Fault("invalid_request", "Invalid JSON") from exc


def obj(value, required=(), optional=()):
    require(isinstance(value, dict), "Expected object")
    require(set(required) <= value.keys() and value.keys() <= set(required) | set(optional),
            "Missing or unknown field")
    return value


def text(value, maximum=2000, minimum=0):
    require(isinstance(value, str) and minimum <= len(value) <= maximum, "Invalid string length")
    require(not any(ord(c) < 32 and c not in "\n\r\t" for c in value), "Control character in string")
    require(not any(0xD800 <= ord(c) <= 0xDFFF for c in value), "Invalid Unicode scalar")
    return value


def integer(value, low=0, high=2**53-1):
    require(type(value) is int and low <= value <= high, "Invalid integer")
    return value


def identifier(value):
    require(isinstance(value, str) and IDENT.fullmatch(value) and value not in FORBIDDEN_IDS, "Invalid schema identifier")
    return value


def uid(value):
    try:
        require(isinstance(value, str) and str(uuid.UUID(value)) == value, "Invalid UUID")
    except (ValueError, AttributeError) as exc:
        raise Fault("invalid_request", "Invalid UUID") from exc
    return value


def new_id():
    return str(uuid.uuid4())


def stamp(value=None):
    return (value or datetime.now(UTC)).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def instant(value):
    require(isinstance(value, str) and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,6})?Z", value),
            "Expected RFC3339 UTC timestamp")
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise Fault("invalid_request", "Invalid UTC timestamp") from exc


def array(value, maximum=100):
    require(isinstance(value, list) and len(value) <= maximum, "Invalid array")
    return value


def refs(value):
    seen = set()
    for ref in array(value, 16):
        obj(ref, ("id", "generation"))
        uid(ref["id"]); integer(ref["generation"], 1)
        require(ref["id"] not in seen, "Duplicate grant")
        seen.add(ref["id"])
    return value


def budgets(value):
    obj(value, ("maxIterations", "maxToolCalls", "maxOutputTokens", "maxDurationSeconds"))
    for key, val in value.items():
        integer(val, 0 if key == "maxToolCalls" else 1, LIMITS[key])
    return value


def history(value):
    for message in array(value, 100):
        obj(message, ("role", "content"))
        require(message["role"] in ("user", "assistant"), "History role forbidden")
        text(message["content"], 16000)
    require(len(canonical(value).encode()) <= 131072, "History too large")
    return value


def runtime_history(value):
    """Strict replay projection for the pinned text/tool-call Hermes adapter.

    System/developer/multimodal/opaque provider authority is never durable worker
    output. Known pinned timestamp/reasoning/finish metadata is validated, then
    excluded unless required for OpenAI-compatible reasoning replay.
    """
    messages = []; pending = {}; seen = set()
    for message in array(value, 1000):
        require(isinstance(message, dict), "Invalid Hermes message")
        role = message.get("role")
        require(role in ("user", "assistant", "tool"), "Hermes history role forbidden")
        fields = ("timestamp",)
        if role == "assistant": fields += ("tool_calls", "reasoning", "reasoning_content", "finish_reason")
        if role == "tool": fields += ("name", "tool_name")
        obj(message, ("role", "content", "tool_call_id") if role == "tool" else ("role", "content"), fields)
        if "timestamp" in message:
            timestamp = message["timestamp"]
            require(type(timestamp) in (float, int) and math.isfinite(timestamp) and 0 <= timestamp <= 1e11, "Invalid message timestamp")
        content = message["content"]
        if content is None: require(role == "assistant" and bool(message.get("tool_calls")), "Missing message content")
        else:
            text(content, 262144 if role == "tool" else 16000 if role == "user" else 64000)
            require(len(content.encode()) <= (262144 if role == "tool" else 65536), "Message byte budget exceeded")
        normalized = {"role": role, "content": content}
        if role != "tool": require(not pending, "Missing tool result in Hermes history")
        if role == "user": seen.clear()  # Provider call IDs are scoped to a user turn.
        if role == "assistant":
            if "finish_reason" in message and message["finish_reason"] is not None: text(message["finish_reason"], 100)
            for key in ("reasoning", "reasoning_content"):
                if message.get(key) is not None:
                    text(message[key], 64000)
                    require(len(message[key].encode()) <= 65536, "Reasoning byte budget exceeded")
            if message.get("reasoning_content") is not None: normalized["reasoning_content"] = message["reasoning_content"]
            if "tool_calls" in message:
                calls = array(message["tool_calls"], 12); require(bool(calls), "Empty tool call list")
                normalized["tool_calls"] = []
                for call in calls:
                    obj(call, ("id", "type", "function"), ("call_id", "response_item_id")); text(call["id"], 200, 1)
                    if "call_id" in call: require(call["call_id"] == call["id"], "Tool call alias mismatch")
                    if "response_item_id" in call:
                        text(call["response_item_id"], 200, 1)
                        require(re.fullmatch(r"[A-Za-z0-9_-]+", call["response_item_id"]), "Invalid response item ID")
                    require(re.fullmatch(r"[A-Za-z0-9_-]+", call["id"]) and call["id"] not in seen and call["type"] == "function", "Invalid tool call identity")
                    function = obj(call["function"], ("name", "arguments"))
                    name = text(function["name"], 100, 1)
                    require(re.fullmatch(r"[A-Za-z0-9_-]+", name), "Invalid history tool name")
                    text(function["arguments"], 65536)
                    require(len(function["arguments"].encode()) <= 65536 and isinstance(decode(function["arguments"]), dict), "Invalid tool arguments")
                    pending[call["id"]] = name; seen.add(call["id"])
                    normalized["tool_calls"].append({"id": call["id"], "type": "function", "function": dict(function)})
        elif role == "tool":
            call_id = text(message["tool_call_id"], 200, 1)
            require(call_id in pending, "Orphan or duplicate tool result")
            name = pending.pop(call_id)
            for key in ("name", "tool_name"):
                if key in message: require(message[key] == name, "Tool result name mismatch")
            # Forbidden builtin attempts may appear in the real loop only as
            # exact denied results. They confer no tool authority on replay.
            if name not in TOOLS: require(decode(content) == {"error": "tool_denied"}, "Forbidden history tool result")
            normalized.update(tool_call_id=call_id, name=name)
        messages.append(normalized)
    require(not pending, "Incomplete history tool exchange")
    require(len(canonical(messages).encode()) <= 2_000_000, "Hermes history too large")
    return messages


def worker_frame(value):
    """Worker output is not a source of arbitrary error codes or DTO fields."""
    try:
        require(isinstance(value, dict), "Expected worker frame")
        kind = value.get("type")
        if kind == "delta":
            obj(value, ("type", "text")); text(value["text"], 16000)
        elif kind == "tool":
            obj(value, ("type", "name", "args")); text(value["name"], 100, 1)
            require(isinstance(value["args"], dict), "Expected tool argument object")
        elif kind == "result":
            obj(value, ("type", "result"))
            obj(value["result"], ("completed", "messages", "final_response"), ("failed", "interrupted", "error", "api_calls"))
            for key in ("completed", "failed", "interrupted"):
                if key in value["result"]: require(type(value["result"][key]) is bool, "Invalid completion flag")
        elif kind == "failure":
            obj(value, ("type", "code")); require(value["code"] == "hermes_worker_failed", "Unapproved worker error code")
        else: raise Fault("worker_protocol_failed", "Invalid worker frame", 409)
    except Fault as exc:
        raise Fault("worker_protocol_failed", "Invalid worker frame", 409) from exc
    return value


def completed_history(value, run):
    messages = runtime_history(value)
    prior = runtime_history(run["_history"])
    require(messages[:len(prior)] == prior, "Worker changed committed history")
    current = messages[len(prior):]
    require(len(current) >= 2 and current[0] == {"role": "user", "content": run["_input"]["message"]}
            and all(m["role"] != "user" for m in current[1:]), "Worker changed operator message")
    require(current[-1]["role"] == "assistant" and not current[-1].get("tool_calls"), "History lacks final assistant message")
    require(sum(len(m.get("tool_calls", [])) for m in current) <= run["budgets"]["maxToolCalls"], "History tool budget exceeded")
    require(sum(m["role"] == "assistant" for m in current) <= run["budgets"]["maxIterations"], "History iteration budget exceeded")
    return messages


def run_input(value):
    obj(value, ("workspaceId", "message", "modelId", "grantRefs", "interfaceIds", "budgets"), ("importHistory",))
    uid(value["workspaceId"]); text(value["message"], 16000, 1); text(value["modelId"], 200, 1)
    refs(value["grantRefs"]); budgets(value["budgets"])
    for item in array(value["interfaceIds"], 20):
        uid(item)
    require(len(set(value["interfaceIds"])) == len(value["interfaceIds"]), "Duplicate interface")
    if "importHistory" in value:
        history(value["importHistory"])
    return value


def interface_spec(value):
    obj(value, ("schemaVersion", "kind", "root", "datasets"))
    require(type(value["schemaVersion"]) is int and value["schemaVersion"] == 1 and value["kind"] == "components",
            "Unsupported interface version")
    pending = [(value, 0)]; seen_objects = set(); entries = 0
    while pending:
        item, depth = pending.pop(); entries += 1
        require(entries <= 6000 and depth <= 16, "Interface structural limit exceeded")
        if isinstance(item, (dict, list)):
            require(id(item) not in seen_objects and len(item) <= 100, "Repeated or oversized structure")
            seen_objects.add(id(item))
            if isinstance(item, dict): require(not FORBIDDEN_IDS.intersection(item), "Forbidden property")
            pending.extend((v, depth + 1) for v in (item.values() if isinstance(item, dict) else item))
    require(len(canonical(value).encode()) <= 65536, "Interface exceeds 64 KiB")
    datasets = {}; dataset_rows = {}
    for dataset in array(value["datasets"], 12):
        obj(dataset, ("id", "columns", "rows")); did = identifier(dataset["id"])
        require(did not in datasets, "Duplicate dataset ID")
        columns = {}
        for column in array(dataset["columns"], 12):
            obj(column, ("id", "type")); cid = identifier(column["id"])
            require(cid not in columns and column["type"] in ("text", "number", "timestamp"), "Invalid column")
            columns[cid] = column["type"]
        require(bool(columns), "Dataset needs columns")
        for row in array(dataset["rows"], 100):
            obj(row, columns)
            for key, item in row.items():
                if item is None:
                    continue
                kind = columns[key]
                if kind == "number":
                    require(type(item) in (int, float) and math.isfinite(item) and abs(item) <= 1e12,
                            "Invalid numeric cell")
                elif kind == "timestamp":
                    require(isinstance(item, str) and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,3})?Z", item), "Invalid interface timestamp")
                    instant(item)
                else:
                    text(item)
        datasets[did] = columns
        dataset_rows[did] = dataset["rows"]
    seen = set(); counts = [0, 0]

    def walk(node, layout_depth):
        require(isinstance(node, dict), "Expected node")
        nid = identifier(node.get("id")); kind = node.get("type")
        require(nid not in seen, "Duplicate node ID"); seen.add(nid)
        counts[0] += 1; require(counts[0] <= 40, "Too many nodes")
        if kind in ("stack", "grid", "card"):
            required = ["id", "type", "children"] + (["columns"] if kind == "grid" else ["title"] if kind == "card" else [])
            obj(node, required)
            require(layout_depth < 3, "Layout depth exceeds three")
            if kind == "grid": integer(node["columns"], 1, 4)
            if "title" in node: text(node["title"], 160)
            for child in array(node["children"], 40): walk(child, layout_depth + 1)
            return
        counts[1] += 1; require(counts[1] <= 12, "Too many content nodes")
        if kind == "text":
            obj(node, ("id", "type", "text")); text(node["text"]); return
        require(kind in ("metric", "table", "chart"), "Unknown node type")
        did = identifier(node.get("datasetId")); require(did in datasets, "Missing dataset")
        columns = datasets[did]
        if kind == "metric":
            obj(node, ("id", "type", "label", "datasetId", "field", "format"), ("currency",))
            text(node["label"], 160); require(columns.get(identifier(node["field"])) == "number", "Metric needs numeric field")
            require(len(dataset_rows[did]) <= 1, "Metric requires one explicit value, not an implicit aggregate")
            require(node["format"] in ("number", "percent", "currency"), "Invalid metric format")
            if node["format"] == "currency":
                require(isinstance(node.get("currency"), str) and re.fullmatch("[A-Z]{3}", node["currency"]), "Invalid currency")
            else: require("currency" not in node, "Currency only valid for currency format")
        elif kind == "table":
            obj(node, ("id", "type", "datasetId", "columns")); used = set()
            require(bool(node["columns"]), "Table needs columns")
            for col in array(node["columns"], 12):
                obj(col, ("field", "label")); text(col["label"], 160)
                require(identifier(col["field"]) in columns and col["field"] not in used,
                        "Invalid table field"); used.add(col["field"])
        else:
            obj(node, ("id", "type", "kind", "datasetId", "xField", "yField", "label", "units"))
            require(node["kind"] in ("bar", "line"), "Invalid chart kind")
            require(columns.get(identifier(node["xField"])) == ("text" if node["kind"] == "bar" else "timestamp")
                    and columns.get(identifier(node["yField"])) == "number", "Invalid chart fields")
            xs = set(); previous = None
            for row in dataset_rows[did]:
                x = row[node["xField"]]
                require(x is not None and x not in xs, "Chart x values must be present and unique"); xs.add(x)
                if node["kind"] == "line":
                    at = instant(x); require(previous is None or at > previous, "Line timestamps must increase"); previous = at
            text(node["label"], 160); text(node["units"], 80)
    try:
        walk(value["root"], 0)
    except (TypeError, RecursionError) as exc:
        raise Fault("invalid_request", "Invalid interface graph") from exc
    return value


def proposal_input(value):
    obj(value, ("workspaceId", "expectedRevision", "title", "spec"), ("interfaceId",))
    uid(value["workspaceId"]); integer(value["expectedRevision"]); text(value["title"], 160, 1)
    interface_spec(value["spec"])
    if "interfaceId" in value:
        uid(value["interfaceId"]); integer(value["expectedRevision"], 1)
    else: require(value["expectedRevision"] == 0, "New proposal revision must be zero")
    return value


class Cron:
    """Five-field numeric UTC cron; no scheduler subprocess or eval."""
    def __init__(self, expression):
        text(expression, 120, 1)
        parts = expression.split()
        require(len(parts) == 5, "Cron needs five fields")
        self.unrestricted = [part == "*" for part in parts]
        self.fields = [self._field(part, low, high) for part, (low, high) in
                       zip(parts, ((0, 59), (0, 23), (1, 31), (1, 12), (0, 7)))]
        if 7 in self.fields[4]: self.fields[4].add(0); self.fields[4].discard(7)

    @staticmethod
    def _field(part, low, high):
        require(re.fullmatch(r"[0-9*,/\-]+", part), "Invalid cron field")
        values = set()
        try:
            for term in part.split(","):
                sections = term.split("/"); require(len(sections) <= 2, "Invalid cron step")
                step = int(sections[1]) if len(sections) == 2 else 1
                require(1 <= step <= high + 1, "Invalid cron step")
                base = sections[0]
                if base == "*": begin, end = low, high
                elif "-" in base:
                    edges = base.split("-"); require(len(edges) == 2, "Invalid cron range")
                    begin, end = map(int, edges)
                else:
                    begin = int(base); end = high if len(sections) == 2 else begin
                require(low <= begin <= end <= high, "Cron value out of range")
                values.update(range(begin, end + 1, step))
        except ValueError as exc:
            raise Fault("invalid_request", "Invalid cron number") from exc
        require(bool(values), "Empty cron field")
        return values

    def matches(self, at):
        minute, hour, day, month, weekday = self.fields
        dom, dow = at.day in day, (at.weekday() + 1) % 7 in weekday
        day_ok = dom and dow if self.unrestricted[2] or self.unrestricted[4] else dom or dow
        return at.minute in minute and at.hour in hour and at.month in month and day_ok

    def next(self, after, end):
        at = after.replace(second=0, microsecond=0) + timedelta(minutes=1)
        ceiling = min(end, after + timedelta(days=366))
        while at < ceiling:
            if self.matches(at): return stamp(at)
            at += timedelta(minutes=1)
        return None


def schedule_input(value):
    obj(value, ("workspaceId", "interfaceId", "expectedInterfaceRevision", "prompt", "modelId", "cron",
                "timezone", "endAt", "maxRuns", "budgets", "grantRefs"))
    uid(value["workspaceId"]); uid(value["interfaceId"]); integer(value["expectedInterfaceRevision"], 1)
    text(value["prompt"], 16000, 1); text(value["modelId"], 200, 1)
    Cron(value["cron"]); require(value["timezone"] == "UTC", "Only UTC schedules supported")
    integer(value["maxRuns"], 1, 100); budgets(value["budgets"]); refs(value["grantRefs"])
    require(instant(value["endAt"]) > datetime.now(UTC), "Schedule endAt must be future")
    return value


def google_args(value):
    obj(value, ("startAt", "endAt", "maxItems"))
    start, end = instant(value["startAt"]), instant(value["endAt"])
    require(timedelta(0) < end - start <= timedelta(days=7), "Read window exceeds seven days")
    integer(value["maxItems"], 1, 100)
    return value


def google_result(data, request):
    obj(data, ("kind", "retrievedAt", "expiresAt", "startAt", "endAt", "truncated", "partial", "items"))
    gmail = request["toolName"] == GOOGLE_TOOLS[0]
    require(data["kind"] == ("gmailMetadata" if gmail else "calendarEvents"), "Wrong result kind")
    args = request["args"]
    require(data["startAt"] == args["startAt"] and data["endAt"] == args["endAt"], "Wrong result window")
    retrieved, expires = instant(data["retrievedAt"]), instant(data["expiresAt"])
    require(retrieved <= datetime.now(UTC) + timedelta(minutes=1) and retrieved < expires <= retrieved + timedelta(days=7),
            "Invalid result freshness")
    require(expires > datetime.now(UTC), "Result expired")
    require(type(data["truncated"]) is bool and type(data["partial"]) is bool, "Invalid result flags")
    for item in array(data["items"], args["maxItems"]):
        if gmail:
            obj(item, ("id", "sender", "subject", "receivedAt", "unread", "sourceUrl"))
            text(item["sender"]); text(item["subject"])
            require(type(item["unread"]) is bool, "Invalid unread state")
            require(instant(args["startAt"]) <= instant(item["receivedAt"]) < instant(args["endAt"]), "Message outside window")
        else:
            obj(item, ("id", "title", "startAt", "endAt", "allDay"), ("sourceUrl",))
            text(item["title"]); require(type(item["allDay"]) is bool, "Invalid allDay flag")
            require(instant(item["startAt"]) < instant(item["endAt"]), "Invalid event interval")
            require(instant(item["startAt"]) < instant(args["endAt"]) and instant(item["endAt"]) > instant(args["startAt"]),
                    "Event outside window")
        text(item["id"], 256, 1)
        if "sourceUrl" in item:
            text(item["sourceUrl"], 2000, 1); url = urlsplit(item["sourceUrl"])
            require(url.scheme == "https" and url.hostname == ("mail.google.com" if gmail else "calendar.google.com")
                    and not url.username and not url.password and url.port in (None, 443)
                    and not any(ord(c) < 33 for c in item["sourceUrl"]), "Invalid source URL")
    require(len(canonical(data).encode()) <= LIMITS["maxToolResultBytes"], "Tool result too large")
    return data
