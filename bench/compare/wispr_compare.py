#!/usr/bin/env python3
"""Head-to-head against Wispr Flow on identical input.

Wispr Flow keeps, for each dictation, the raw speech-recognition text and the
text it finally inserted (History.asrText / formattedText / pastedText in
~/Library/Application Support/Wispr Flow/flow.sqlite). Feeding its raw text to
our prompt and comparing the result with what it inserted isolates the
formatting step: same words in, two formatters out.

  python3 bench/compare/wispr_compare.py export.json --model anthropic/claude-haiku-5.5

`export.json` is a list of {"raw", "formatted" and/or "pasted", ...}. It holds
real dictations, so it stays out of the repository. Results are written next
to it as <name>.results.json; the summary printed here contains no text.
"""
import argparse
import concurrent.futures as cf
import difflib
import html
import importlib.util
import json
import pathlib
import re
import statistics

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("fmt", HERE.parent / "formatting" / "run.py")
fmt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fmt)


def html_to_text(text):
    """Wispr Flow stores lists as HTML; render them the way it pastes them."""
    if "<" not in text:
        return text.strip()

    def ordered(match):
        items = re.findall(r"<li[^>]*>(.*?)</li>", match.group(1), re.S)
        return "\n" + "\n".join(f"{n}. {item.strip()}" for n, item in enumerate(items, 1)) + "\n"

    def unordered(match):
        items = re.findall(r"<li[^>]*>(.*?)</li>", match.group(1), re.S)
        return "\n" + "\n".join(f"- {item.strip()}" for item in items) + "\n"

    text = re.sub(r"<ol[^>]*>(.*?)</ol>", ordered, text, flags=re.S)
    text = re.sub(r"<ul[^>]*>(.*?)</ul>", unordered, text, flags=re.S)
    text = re.sub(r"<br\s*/?>", "\n", text)
    text = re.sub(r"</p>\s*<p[^>]*>", "\n\n", text)
    text = re.sub(r"<[^>]+>", "", text)
    return html.unescape(text).strip()


def shape(text):
    lines = [line for line in text.splitlines() if line.strip()]
    numbered = sum(bool(fmt.NUMBERED.match(line)) for line in lines)
    bullets = sum(bool(fmt.BULLET.match(line)) for line in lines)
    kind = "numbered" if numbered >= 2 else "bullets" if bullets >= 2 else "none"
    paragraphs = len([p for p in re.split(r"\n\s*\n", text) if p.strip()])
    return {"list": kind, "items": numbered + bullets, "paragraphs": paragraphs}


def words(text):
    return re.findall(r"[\w']+", text.lower())


def similarity(a, b):
    return difflib.SequenceMatcher(None, words(a), words(b), autojunk=False).ratio()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("export")
    parser.add_argument("--model", default="anthropic/claude-haiku-5.5")
    parser.add_argument("--prompt", default="flow")
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()

    path = pathlib.Path(args.export)
    rows = [row for row in json.loads(path.read_text()) if (row.get("raw") or "").strip()]
    if args.limit:
        rows = rows[: args.limit]
    prompt = (HERE.parent / "formatting" / "prompts" / f"{args.prompt}.txt").read_text()
    prompt = prompt.replace("${output}", "").strip()
    key = fmt.api_key()

    with cf.ThreadPoolExecutor(4) as pool:
        replies = list(pool.map(lambda row: fmt.complete(key, args.model, prompt, row["raw"]), rows))

    results = []
    for row, (ours, seconds, error) in zip(rows, replies):
        theirs = html_to_text(row.get("pasted") or row.get("formatted") or "")
        if error or not theirs:
            continue
        ours = ours.strip()
        results.append(
            {
                "id": row.get("id"),
                "raw": row["raw"],
                "wispr": theirs,
                "ours": ours,
                "wispr_shape": shape(theirs),
                "our_shape": shape(ours),
                "similarity": round(similarity(ours, theirs), 3),
                "raw_similarity_wispr": round(similarity(row["raw"], theirs), 3),
                "raw_similarity_ours": round(similarity(row["raw"], ours), 3),
                "length_ratio": round(len(ours) / max(1, len(theirs)), 3),
                "our_seconds": round(seconds, 3),
                "wispr_e2e_ms": row.get("e2eLatencyMs") or row.get("e2eLatency"),
                "words": len(words(row["raw"])),
            }
        )
    out = path.with_suffix(".results.json")
    out.write_text(json.dumps(results, indent=1, ensure_ascii=False))

    n = len(results)
    both_list = sum(r["wispr_shape"]["list"] != "none" and r["our_shape"]["list"] != "none" for r in results)
    only_wispr = sum(r["wispr_shape"]["list"] != "none" and r["our_shape"]["list"] == "none" for r in results)
    only_ours = sum(r["wispr_shape"]["list"] == "none" and r["our_shape"]["list"] != "none" for r in results)
    same_kind = sum(
        r["wispr_shape"]["list"] == r["our_shape"]["list"] for r in results if r["wispr_shape"]["list"] != "none"
    )
    wispr_lists = both_list + only_wispr
    same_paragraphs = sum(r["wispr_shape"]["paragraphs"] == r["our_shape"]["paragraphs"] for r in results)
    sims = sorted(r["similarity"] for r in results)
    ours_s = sorted(r["our_seconds"] for r in results)
    theirs_ms = sorted(r["wispr_e2e_ms"] for r in results if r["wispr_e2e_ms"])
    pct = lambda values, q: values[min(len(values) - 1, int(len(values) * q))]
    print(f"{n} dictations, model {args.model}, prompt {args.prompt}")
    print(f"  list where Wispr made one:   {both_list}/{wispr_lists} (same kind: {same_kind}/{wispr_lists})")
    print(f"  list where Wispr made none:  {only_ours}/{n - wispr_lists}")
    print(f"  same paragraph count:        {same_paragraphs}/{n}")
    print(f"  word similarity to Wispr:    p10={pct(sims, 0.1):.2f} p50={statistics.median(sims):.2f} p90={pct(sims, 0.9):.2f}")
    print(
        "  edit vs raw (1.0=untouched): wispr p50={:.2f}  ours p50={:.2f}".format(
            statistics.median(r["raw_similarity_wispr"] for r in results),
            statistics.median(r["raw_similarity_ours"] for r in results),
        )
    )
    print(f"  length ours/Wispr:           p50={statistics.median(r['length_ratio'] for r in results):.2f}")
    total_words = sum(r["words"] for r in results) or 1
    per100 = lambda key, pattern: 100 * sum(len(re.findall(pattern, r[key])) for r in results) / total_words
    for label, pattern in (("commas", ","), ("digit tokens", r"\d[\d,.]*"), ("hyphenated", r"\w-\w"), ("quotes", '["“”]')):
        print(f"  {label + ' per 100 words:':<29}wispr {per100('wispr', pattern):.2f}  ours {per100('ours', pattern):.2f}")
    print(f"  our formatting request:      p50={statistics.median(ours_s):.2f}s p90={pct(ours_s, 0.9):.2f}s")
    if theirs_ms:
        print(
            f"  Wispr end-to-end (recorded): p50={statistics.median(theirs_ms):.0f}ms p90={pct(theirs_ms, 0.9):.0f}ms"
        )
    print(f"  details: {out}")


if __name__ == "__main__":
    main()
