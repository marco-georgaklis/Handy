#!/usr/bin/env python3
"""Formatting benchmark for the post-processing step.

Sends each raw dictation in cases.json through an OpenAI-compatible endpoint
the same way the app does (prompt as the system message, transcript as the
user message, structured output with a `transcription` field, reasoning off)
and scores the reply with deterministic checks. Reports pass rate per case
category and request latency per prompt x model.

  python3 bench/formatting/run.py --prompts current flow --models openai/gpt-4.1-mini

The API key comes from $OPENROUTER_API_KEY, else from the installed app's
settings (the key configured for the selected provider). It is never printed.
"""
import argparse
import concurrent.futures as cf
import json
import os
import pathlib
import re
import ssl
import statistics
import sys
import time
import urllib.error
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
SETTINGS = pathlib.Path.home() / "Library/Application Support/com.handyflow.app/settings_store.json"
BASE_URL = "https://openrouter.ai/api/v1"
SCHEMA = {
    "type": "object",
    "properties": {
        "transcription": {
            "type": "string",
            "description": "The cleaned and processed transcription text",
        }
    },
    "required": ["transcription"],
    "additionalProperties": False,
}
# python.org builds ship without a CA bundle; fall back to the system one.
TLS = ssl.create_default_context()
if not TLS.get_ca_certs() and os.path.exists("/etc/ssl/cert.pem"):
    TLS = ssl.create_default_context(cafile="/etc/ssl/cert.pem")
NO_REASONING_OFF = set()
BULLET = re.compile(r"^\s*[-*•]\s+\S")
NUMBERED = re.compile(r"^\s*\d+[.)]\s+\S")


def api_key():
    key = os.environ.get("OPENROUTER_API_KEY")
    if key:
        return key
    settings = json.loads(SETTINGS.read_text())["settings"]
    key = settings.get("post_process_api_keys", {}).get("openrouter")
    if not key:
        sys.exit("No OpenRouter key: set OPENROUTER_API_KEY")
    return key


def wrap(system_prompt, transcript):
    """Mirrors the app: a prompt that talks about <dictation> tags gets them."""
    if "<dictation>" in system_prompt:
        return f"<dictation>\n{transcript}\n</dictation>"
    return transcript


def complete(key, model, system_prompt, transcript, timeout=60):
    body = {
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": wrap(system_prompt, transcript)},
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {"name": "transcription_output", "strict": True, "schema": SCHEMA},
        },
    }
    if model not in NO_REASONING_OFF:
        body["reasoning"] = {"effort": "none", "exclude": True}
    request = urllib.request.Request(
        BASE_URL + "/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=timeout, context=TLS) as response:
            payload = json.load(response)
    except urllib.error.HTTPError as error:
        detail = error.read()[:200]
        if error.code in (400, 422) and "reasoning" in body:
            # Like the app: an endpoint that rejects the reasoning-off fields
            # is retried without them (and remembered for the session).
            NO_REASONING_OFF.add(model)
            return complete(key, model, system_prompt, transcript, timeout)
        return None, time.monotonic() - started, f"HTTP {error.code}: {detail!r}"
    except Exception as error:  # timeouts, connection resets
        return None, time.monotonic() - started, repr(error)
    elapsed = time.monotonic() - started
    try:
        content = payload["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError):
        return None, elapsed, f"no content: {json.dumps(payload)[:200]}"
    try:
        content = json.loads(content)["transcription"]
    except (ValueError, KeyError, TypeError):
        pass  # the app also falls back to the raw content
    return content, elapsed, None


def has(text, needle):
    return needle.lower() in text.lower()


def score(case, output):
    """Returns the list of failed checks (empty means the case passed)."""
    failures = []
    lines = [line for line in output.splitlines() if line.strip()]
    bullets = [line for line in lines if BULLET.match(line)]
    numbered = [line for line in lines if NUMBERED.match(line)]
    items = bullets + numbered
    want = case.get("list", "none")
    minimum = case.get("min_items", 3)
    if want == "none" and items:
        failures.append("made a list out of prose")
    if want == "bullets" and len(bullets) < minimum:
        failures.append(f"expected >={minimum} bullets, got {len(bullets)}")
    if want == "numbered" and len(numbered) < minimum:
        failures.append(f"expected >={minimum} numbered items, got {len(numbered)}")
    if want == "any" and len(items) < minimum:
        failures.append(f"expected a list of >={minimum}, got {len(items)}")
    for needle in case.get("keep", []):
        if not has(output, needle):
            failures.append(f"lost '{needle}'")
    for group in case.get("keep_any", []):
        if not any(has(output, needle) for needle in group):
            failures.append(f"lost one of {group}")
    for needle in case.get("drop", []):
        # Case-sensitive: "Um" at the start and " um" mid-sentence are listed
        # separately so that words like "summary" don't trip the check.
        if needle in output:
            failures.append(f"kept '{needle.strip()}'")
    outside = "\n".join(line for line in lines if line not in items)
    if want == "optional" and items and len(items) < 3:
        failures.append("list of fewer than 3 items")
    for needle in case.get("outside_list", []):
        if not has(outside, needle):
            failures.append(f"'{needle}' should be outside the list")
    paragraphs = [p for p in re.split(r"\n\s*\n", output) if p.strip()]
    if len(paragraphs) < case.get("min_paragraphs", 1) and len(lines) < case.get("min_paragraphs", 1):
        failures.append(f"expected >={case['min_paragraphs']} paragraphs")
    if re.search(r"^#{1,6}\s|\*\*|```", output, re.M):
        failures.append("used markdown headings/bold/code fences")
    ratio = len(output) / max(1, len(case["input"]))
    if ratio > 1.35:
        failures.append(f"added text (x{ratio:.2f})")
    if ratio < 0.35:
        failures.append(f"dropped text (x{ratio:.2f})")
    return failures


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--prompts", nargs="+", default=["current", "flow"])
    parser.add_argument("--models", nargs="+", default=["openai/gpt-4.1-mini"])
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--only", help="run a single case id")
    parser.add_argument("--show", action="store_true", help="print every output")
    parser.add_argument("--json", help="write full results to this file")
    args = parser.parse_args()

    key = api_key()
    cases = json.loads((HERE / "cases.json").read_text())
    if args.only:
        cases = [case for case in cases if case["id"] == args.only]
    results = []
    for prompt_name in args.prompts:
        system_prompt = (HERE / "prompts" / f"{prompt_name}.txt").read_text()
        system_prompt = system_prompt.replace("${output}", "").strip()
        for model in args.models:
            jobs = [case for case in cases for _ in range(args.repeat)]
            with cf.ThreadPoolExecutor(4) as pool:
                replies = list(
                    pool.map(lambda case: complete(key, model, system_prompt, case["input"]), jobs)
                )
            rows = []
            for case, (output, elapsed, error) in zip(jobs, replies):
                failures = [f"request failed: {error}"] if error else score(case, output)
                rows.append(
                    {
                        "prompt": prompt_name,
                        "model": model,
                        "id": case["id"],
                        "kind": case["kind"],
                        "seconds": round(elapsed, 3),
                        "failures": failures,
                        "output": output,
                    }
                )
            results.extend(rows)
            latencies = sorted(row["seconds"] for row in rows)
            passed = sum(not row["failures"] for row in rows)
            kinds = sorted({row["kind"] for row in rows})
            by_kind = " ".join(
                "{}={}/{}".format(
                    kind,
                    sum(not row["failures"] for row in rows if row["kind"] == kind),
                    sum(row["kind"] == kind for row in rows),
                )
                for kind in kinds
            )
            print(
                f"\n== prompt={prompt_name} model={model}: {passed}/{len(rows)} passed"
                f" | latency p50={statistics.median(latencies):.2f}s"
                f" p90={latencies[int(len(latencies) * 0.9) - 1]:.2f}s max={latencies[-1]:.2f}s"
            )
            print(f"   {by_kind}")
            errors = [row for row in rows if row["failures"][:1] and row["failures"][0].startswith("request failed")]
            if len(errors) > 3:
                print(f"   {len(errors)} requests failed, e.g. {errors[0]['failures'][0][:160]}")
                continue
            for row in rows:
                if row["failures"]:
                    print(f"   FAIL {row['id']}: {'; '.join(row['failures'])}")
                if args.show or (row["failures"] and args.only):
                    print("   ---\n   " + (row["output"] or "").replace("\n", "\n   ") + "\n   ---")
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
