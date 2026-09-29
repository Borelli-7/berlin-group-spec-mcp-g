#!/usr/bin/env python3
"""Exercise the real stdio binary and optionally save exact example responses."""
import argparse
import json
from pathlib import Path
import selectors
import subprocess


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=root / "target/release/bg-spec-mcp")
    parser.add_argument("--config", type=Path, default=root / "config/config.toml")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    requests = json.loads((root / "examples/requests.json").read_text())
    process = subprocess.Popen(
        [str(args.binary.resolve()), "--config", str(args.config.resolve())],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        # Server stderr is deliberately inherited, not mixed into protocol output.
    )
    sequence = 0
    buffer = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)

    def send(value):
        process.stdin.write(json.dumps(value).encode() + b"\n")
        process.stdin.flush()

    def rpc(method, params):
        nonlocal sequence
        sequence += 1
        send({"jsonrpc": "2.0", "id": sequence, "method": method, "params": params})
        while True:
            while b"\n" not in buffer:
                if not selector.select(timeout=20):
                    raise TimeoutError(f"No MCP response to {method}")
                block = process.stdout.read1(65536)
                if not block:
                    raise RuntimeError(f"MCP exited before replying to {method}")
                buffer.extend(block)
            line, _, remaining = buffer.partition(b"\n")
            buffer[:] = remaining
            reply = json.loads(line)  # Fails if any log contaminates stdout.
            if reply.get("id") == sequence:
                if "error" in reply:
                    raise RuntimeError(reply["error"])
                return reply["result"]

    try:
        initialized = rpc("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "bg-spec-smoke", "version": "1.0"},
        })
        assert initialized["capabilities"]["tools"] == {}
        send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        tools = rpc("tools/list", {})
        assert {t["name"] for t in tools["tools"]} == {r["name"] for r in requests}
        output = {}
        for request in requests:
            reply = rpc("tools/call", request)
            assert not reply.get("isError", False), reply
            output[request["name"]] = reply["structuredContent"]
        bundle = output["get_endpoint_requirements"]
        assert bundle["requirements"][0]["id"] == "FIXTURE-OFV2-TRANSACTIONS-001"
        assert len(bundle["schemas"]) == 2
        assert bundle["related_evidence"]
        assert output["compare_v1_v2"]["schema_changes"]
        error = rpc("tools/call", {"name": "read_source", "arguments": {
            "source_id": "../../etc/passwd", "locator": "page:1",
        }})
        assert error["isError"] is True
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n")
        print(f"stdio acceptance passed: initialize, {len(requests)} tools, error handling")
    finally:
        selector.close()
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=5)
        process.stdout.close()


if __name__ == "__main__":
    main()
