"""Turns voicemirror_eval.json into the markdown tables used in
docs/phase3/PHASE3_EVALUATION_REPORT.md (so reported numbers are generated,
not transcribed).  Usage: python summarize.py path/to/voicemirror_eval.json"""
import json, sys, math
from collections import defaultdict

r = json.load(open(sys.argv[1]))
ident = r["identity"]
CONDS = ["baseline", "vm_prosody", "vm_register", "oracle_prosody", "oracle_select", "oracle_full"]
f = lambda x, n=3: "—" if x is None or (isinstance(x, float) and math.isnan(x)) else f"{x:.{n}f}"

print("### Overall (all language pairs)\n")
print(f"{ident['n_speakers']} speakers × {ident['pairs']} ordered pairs; descriptor coverage {ident['descriptor_coverage']:.0%}; "
      f"different-speaker cosine (reference) {ident['overall']['_impostor_cos_mean']:.3f}\n")
print("| Condition | n | Cosine mean | Top-1 ident. | Mean rank | Pitch err median (st) | Pitch err p90 | Tempo err median (%) | Tempo err p90 |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
for c in CONDS:
    o = ident["overall"].get(c)
    if not o: continue
    print(f"| `{c}` | {o['n']} | {f(o['cos']['mean'])} | {o['top1_rate']:.1%} | {o['mean_rank']:.1f} | "
          f"{f(o['pitch_err_st'].get('median'),2)} | {f(o['pitch_err_st'].get('p90'),2)} | {f(o['tempo_err_pct']['median'],1)} | {f(o['tempo_err_pct']['p90'],1)} |")

print("\n### Per target language (mean over source languages)\n")
by_tgt = defaultdict(lambda: defaultdict(list))
for pair, s in ident["by_pair"].items():
    tgt = pair.split("->")[1]
    for c in CONDS:
        if c in s:
            by_tgt[tgt][c].append((s[c]["cos"]["mean"], s[c]["pitch_err_st"].get("median", float("nan")), s[c]["top1_rate"]))
print("| Target | " + " | ".join(f"{c} cos / pitch-err st" for c in ["baseline", "vm_prosody", "vm_register", "oracle_full"]) + " |")
print("| --- " * 5 + "|")
for tgt in sorted(by_tgt):
    cells = []
    for c in ["baseline", "vm_prosody", "vm_register", "oracle_full"]:
        v = by_tgt[tgt][c]
        cells.append(f"{sum(x[0] for x in v)/len(v):.3f} / {sum(x[1] for x in v)/len(v):.2f}")
    print(f"| {tgt} | " + " | ".join(cells) + " |")

print("\n### Per pair: vm_prosody − baseline (cosine Δ, pitch-error Δ st)\n")
print("| Pair | Δ cosine | Δ pitch err | vm_prosody top-1 |")
print("| --- | --- | --- | --- |")
for pair, s in sorted(ident["by_pair"].items()):
    if "vm_prosody" in s:
        dc = s["vm_prosody"]["cos"]["mean"] - s["baseline"]["cos"]["mean"]
        dp = s["vm_prosody"]["pitch_err_st"]["median"] - s["baseline"]["pitch_err_st"]["median"]
        print(f"| {pair} | {dc:+.3f} | {dp:+.2f} | {s['vm_prosody']['top1_rate']:.0%} |")

# Tempo reliability by speaker tempo
rows = [x for x in r["identity_rows"] if x.get("cond") == "vm_prosody"]
if rows:
    import statistics as st
    print("\n### vm_prosody tempo error by true speaker tempo\n")
    sp = r["meta"]["eval_prosody"]
    print("| True speed (wpm) | n | median tempo err (%) |")
    print("| --- | --- | --- |")
    for (p, s) in sp:
        idx = [i for i, (pp, ss) in enumerate([(a, b) for v in r["meta"]["eval_variants"] for (a, b) in sp]) if ss == s]
        errs = [x["tempo_err_pct"] for x in rows if x["spk"] in idx]
        print(f"| {s} | {len(errs)} | {st.median(errs):.1f} |")

print("\n### Calibration\n")
c = r["calibration"]
print(f"- reference syllable rate at 175 wpm: {c['reference_syllable_rate_at_175wpm']:.2f} (product constant {c['product_reference_rate']}; matches: {c['product_matches_calibration']})")
print(f"- tempo rank correlation (English, calibration speakers): {c['tempo_corr_spearman_en']:.3f}")
print(f"- voiced seconds per single sentence: median {c['single_sentence_voiced_s']['median']:.2f}")

print("\n### Analyzer accuracy (exact synthetic tones)\n")
e = r["extraction"]
print(f"- pitch abs error: median {e['tone_abs_err_semitones']['median']:.3f} st, max {e['tone_abs_err_semitones']['max']:.3f} st (n={e['tone_abs_err_semitones']['n']}, 80–350 Hz, 16/48 kHz)")

print("\n### Intelligibility proxy (pocketsphinx WER, English targets; relative only)\n")
ip = r.get("intelligibility_proxy", {})
if "by_condition" in ip:
    print("| Condition | n | WER mean | WER median |")
    print("| --- | --- | --- | --- |")
    for c, v in ip["by_condition"].items():
        print(f"| `{c}` | {v['n']} | {v['mean']:.2f} | {v['median']:.2f} |")
    print("\n| Parameter extreme (default voice) | WER mean |")
    print("| --- | --- |")
    for k, v in ip["param_extremes"].items():
        print(f"| {k} | {v['mean']:.2f} |")
else:
    print(ip)

print("\n### Robustness (analyzer → descriptor)\n")
rb = r["robustness"]
print("| Input | Descriptor produced | Same pitch step as clean | Pitch drift vs clean (median st) |")
print("| --- | --- | --- | --- |")
for k, v in rb.items():
    if isinstance(v, dict):
        same = "—" if v["same_pitch_step_rate"] is None else f"{v['same_pitch_step_rate']:.0%}"
        print(f"| {k} | {v['descriptor_rate']:.0%} | {same} | {f(v['pitch_err_vs_clean_st'].get('median'), 2)} |")
print(f"| clean | {rb['clean_descriptor_rate']:.0%} | — | — |")

print("\n### Resources\n")
res = r["resources"]
print(f"- analyzer: {res['analyzer_ms_per_audio_second_node']:.2f} ms per audio second (Node {r['meta']['node']}, {res['cpu']} vCPU, no GPU; {res['audio_seconds']:.0f} s at {res['sample_rate']} Hz)")
print(f"- descriptor: {res['descriptor_json_bytes']} bytes JSON")
print(f"\nRun: {r['meta']['started']}, {r['meta']['elapsed_s']/60:.1f} min, {r['meta']['espeak']}")
