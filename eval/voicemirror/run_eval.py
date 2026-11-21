"""VoiceMirror evaluation harness (Phase 3, M3.3).

Synthetic voices only (espeak-ng). The PRODUCT analyzer/descriptor code
(src/voicemirror, via eval/voicemirror/extract.ts under Node) produces the
voice descriptors that are evaluated; Python only renders audio and scores it.

Stages
  1. calibrate   — on CALIBRATION speakers only: espeak pitch-param → F0 map for
                   the render voices; tempo reference; confidence rule.
  2. extraction  — analyzer accuracy vs exact synthetic tones and vs librosa pYIN.
  3. identity    — EVALUATION speakers × language pairs × conditions:
                   baseline (fixed voice, today), vm_prosody (descriptor → pitch+rate),
                   vm_register (descriptor → stock voice by register + speed, the
                   server-TTS path), oracle_prosody, oracle_select, oracle_full.
                   Speaker-embedding cosine (Resemblyzer GE2E), top-1 identification,
                   pitch error and tempo error, separately.
  4. intelligibility proxy — pocketsphinx WER on English targets (relative only).
  5. robustness  — noise, short, silence, clipping, sample rates.
  6. resources   — analyzer time per audio second, state size, descriptor bytes.

Usage:  python run_eval.py OUT_DIR [--quick]
Requires: espeak-ng, node>=22, numpy, librosa, resemblyzer, pocketsphinx.
"""
import json, math, os, subprocess, sys, tempfile, time, wave, itertools, hashlib
from concurrent.futures import ThreadPoolExecutor
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "results")
QUICK = "--quick" in sys.argv
os.makedirs(OUT, exist_ok=True)
TMP = tempfile.mkdtemp(prefix="vm_eval_")
RNG = np.random.default_rng(20261121)

from corpus import SENTENCES, ESPEAK_LANG  # noqa: E402

# ----------------------------------------------------------------- speakers
CAL_VARIANTS = ["m2", "m4", "f2", "f4"]
EVAL_VARIANTS = ["m1", "m3", "m7", "f1", "f3", "f5"]
EVAL_PROSODY = [(25, 140), (50, 175), (75, 205)]  # (espeak pitch, speed wpm)
DEFAULT_VOICE = dict(v="m1", p=50, s=175)  # "today": one fixed voice
# Stock-voice catalog for the register path (disjoint from evaluation speakers).
CATALOG = {"low": "m4", "mid": "m2", "high": "f2"}
LANGS = ["en", "es", "de", "hi", "ja", "ru"]
if QUICK:
    LANGS = ["en", "es", "hi"]
    EVAL_PROSODY = [(25, 140), (75, 205)]

# ----------------------------------------------------------------- utilities
def render(v, p, s, lang, text, tag):
    key = hashlib.sha1(f"{v}|{p}|{s}|{lang}|{text}".encode()).hexdigest()[:16]
    path = os.path.join(TMP, f"{tag}_{key}.wav")
    if not os.path.exists(path):
        subprocess.run(["espeak-ng", "-v", f"{ESPEAK_LANG[lang]}+{v}", "-p", str(int(round(p))), "-s", str(int(round(s))),
                        "-w", path, text], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return path

def read_wav(path):
    with wave.open(path, "rb") as w:
        n = w.getnframes(); sr = w.getframerate()
        x = np.frombuffer(w.readframes(n), dtype=np.int16).astype(np.float32) / 32768
    return x, sr

def write_wav(path, x, sr):
    x = np.clip(x, -1, 1)
    with wave.open(path, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr)
        w.writeframes((x * 32767).astype(np.int16).tobytes())
    return path

def concat(paths, tag):
    xs, sr = [], None
    for p in paths:
        x, sr = read_wav(p); xs.append(x); xs.append(np.zeros(int(0.15 * sr), np.float32))
    return write_wav(os.path.join(TMP, f"cat_{tag}.wav"), np.concatenate(xs), sr)

def extract(paths):
    """Runs the product TypeScript analyzer over WAVs (batched)."""
    out = []
    for i in range(0, len(paths), 200):
        r = subprocess.run(["node", "--experimental-strip-types", os.path.join(HERE, "extract.ts"), *paths[i:i + 200]],
                           capture_output=True, text=True, check=True)
        out += json.loads(r.stdout)
    return out

_pyin_cache = {}
def pyin_median(path):
    if path not in _pyin_cache:
        import librosa
        x, sr = read_wav(path)
        f0, vf, _ = librosa.pyin(x, fmin=55, fmax=550, sr=sr, frame_length=1024)
        f0 = f0[vf & ~np.isnan(f0)]
        _pyin_cache[path] = float(np.median(f0)) if len(f0) else float("nan")
    return _pyin_cache[path]

st = lambda a, b: 12 * math.log2(a / b)
def stats(xs):
    xs = np.asarray([x for x in xs if x is not None and not (isinstance(x, float) and math.isnan(x))], float)
    if not len(xs): return {"n": 0}
    return {"n": int(len(xs)), "mean": float(xs.mean()), "median": float(np.median(xs)),
            "p90": float(np.percentile(xs, 90)), "min": float(xs.min()), "max": float(xs.max())}

def pool_map(fn, items, workers=4):
    with ThreadPoolExecutor(workers) as ex:
        return list(ex.map(fn, items))

results = {"meta": {"started": time.strftime("%Y-%m-%dT%H:%M:%S"), "quick": QUICK, "langs": LANGS,
                    "eval_variants": EVAL_VARIANTS, "eval_prosody": EVAL_PROSODY, "cal_variants": CAL_VARIANTS,
                    "catalog": CATALOG, "default_voice": DEFAULT_VOICE,
                    "espeak": subprocess.run(["espeak-ng", "--version"], capture_output=True, text=True).stdout.strip(),
                    "node": subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip()}}
T0 = time.time()

# ============================================================ 1. calibration
print("[1] calibration (calibration speakers only)")
cal_rows = []
cal_items = [(v, p, s, lang, i) for v in CAL_VARIANTS for p in (20, 40, 60, 80) for s in (130, 175, 220)
             for lang in ("en",) for i in range(2)]
cal_paths = pool_map(lambda it: render(it[0], it[1], it[2], it[3], SENTENCES[it[3]][it[4]], "cal"), cal_items)
cal_ex = extract(cal_paths)
for it, path, ex in zip(cal_items, cal_paths, cal_ex):
    cal_rows.append(dict(v=it[0], p=it[1], s=it[2], f0_ref=pyin_median(path), **ex["profile"]))
# (a) pitch param → F0 per render voice (espeak's -p is linear-ish in Hz per variant)
def fit_pitch_map(variant):
    pts = [(r["p"], r["f0_ref"]) for r in cal_rows if r["v"] == variant and not math.isnan(r["f0_ref"])]
    if not pts:  # render voice not in calibration set: measure it directly
        pts = [(p, pyin_median(render(variant, p, 175, "en", SENTENCES["en"][0], "calr"))) for p in (20, 40, 60, 80)]
    P, F = np.array(pts).T
    a, b = np.polyfit(P, np.log2(F), 1)  # log-frequency linear in p
    return {"slope_log2": float(a), "icept_log2": float(b), "points": len(P)}
pitch_maps = {v: fit_pitch_map(v) for v in {DEFAULT_VOICE["v"], *CATALOG.values()}}
def espeak_p_for(variant, f0):
    if variant not in pitch_maps:  # calibrate a render voice on first use (render-voice property, not speaker data)
        pitch_maps[variant] = fit_pitch_map(variant)
    m = pitch_maps[variant]
    return float(np.clip((math.log2(f0) - m["icept_log2"]) / m["slope_log2"], 0, 99))
def espeak_f0_at(variant, p):
    m = pitch_maps[variant]; return 2 ** (m["icept_log2"] + m["slope_log2"] * p)
# (b) tempo: syllable-peak rate vs espeak speed (English calibration)
S = np.array([r["s"] for r in cal_rows]); R = np.array([r["syllableRate"] for r in cal_rows])
ok = R > 0
k_rate_per_wpm = float(np.sum(R[ok] * S[ok]) / np.sum(S[ok] ** 2))  # R ≈ k·S through origin
reference_rate = k_rate_per_wpm * DEFAULT_VOICE["s"]
# (c) confidence: voiced seconds seen in single sentences
voiced = np.array([r["voicedSeconds"] for r in cal_rows])
results["calibration"] = {
    "pitch_maps": pitch_maps, "rate_per_wpm": k_rate_per_wpm, "reference_syllable_rate_at_175wpm": reference_rate,
    "tempo_corr_spearman_en": float(np.corrcoef(np.argsort(np.argsort(S[ok])), np.argsort(np.argsort(R[ok])))[0, 1]),
    "single_sentence_voiced_s": stats(voiced),
}
print("   ", json.dumps(results["calibration"])[:400])
REF_RATE = reference_rate

def descriptor_to_params(d, render_variant):
    """Mirror of src/voicemirror/descriptor.ts mapping for an engine with
    calibrated pitch: target F0 from the descriptor, speed from tempo."""
    target = 55 * 2 ** (d["pitchStep"] / 12)
    return espeak_p_for(render_variant, target), DEFAULT_VOICE["s"] * d["tempoPct"] / 100, target

# The product constant must match calibration — checked, not silently overridden.
prod_ref = float(open(os.path.join(ROOT, "src/voicemirror/descriptor.ts")).read().split("REFERENCE_SYLLABLE_RATE = ")[1].split(";")[0])
results["calibration"]["product_reference_rate"] = prod_ref
results["calibration"]["product_matches_calibration"] = abs(prod_ref - REF_RATE) / REF_RATE < 0.1
if not results["calibration"]["product_matches_calibration"]:
    print(f"    WARNING: src/voicemirror/descriptor.ts REFERENCE_SYLLABLE_RATE={prod_ref} but calibration says {REF_RATE:.2f}")

# ============================================================ 2. extraction accuracy
print("[2] analyzer accuracy")
def tone(f0, secs, sr, vib=0.0):
    t = np.arange(int(secs * sr)) / sr
    f = f0 * 2 ** (vib * np.sin(2 * np.pi * 5 * t) / 12)
    ph = 2 * np.pi * np.cumsum(f) / sr
    x = sum((0.6 / k) * np.sin(k * ph) for k in range(1, 12))  # harmonic, voice-like spectrum
    env = (0.5 - 0.5 * np.cos(2 * np.pi * 4 * t)) ** 2  # 4 Hz syllable-like envelope
    return (0.3 * x * env).astype(np.float32)
tone_rows = []
tone_specs = [(f, sr) for f in (80, 110, 150, 200, 260, 350) for sr in (16000, 48000)]
tone_paths = [write_wav(os.path.join(TMP, f"tone_{f}_{sr}.wav"), tone(f, 3.0, sr, vib=0.5), sr) for f, sr in tone_specs]
for (f, sr), ex in zip(tone_specs, extract(tone_paths)):
    tone_rows.append(dict(true_f0=f, sr=sr, est=ex["profile"]["f0MedianHz"],
                          err_st=abs(st(ex["profile"]["f0MedianHz"], f)) if ex["profile"]["f0MedianHz"] > 0 else None,
                          rate=ex["profile"]["syllableRate"]))
results["extraction"] = {"tones": tone_rows, "tone_abs_err_semitones": stats([r["err_st"] for r in tone_rows]),
                         "tone_syllable_rate_true_4hz": stats([r["rate"] for r in tone_rows])}

# ============================================================ 3. identity
print("[3] identity / pitch / tempo across language pairs")
from resemblyzer import VoiceEncoder, preprocess_wav  # noqa: E402
enc = VoiceEncoder("cpu", verbose=False)
_emb = {}
def embed(paths):
    key = tuple(paths)
    if key not in _emb:
        _emb[key] = enc.embed_speaker([preprocess_wav(p) for p in paths])
    return _emb[key]

speakers = [dict(v=v, p=p, s=s) for v in EVAL_VARIANTS for (p, s) in EVAL_PROSODY]
pairs = [(a, b) for a in LANGS for b in LANGS if a != b]
N_ENR, N_OUT = 3, 2
catalog_variants = EVAL_VARIANTS  # oracle_select: best OTHER timbre among evaluation timbres

rows = []
for (l1, l2) in pairs:
    t_pair = time.time()
    enr_paths = {k: pool_map(lambda i: render(sp["v"], sp["p"], sp["s"], l1, SENTENCES[l1][i], "enr"), range(N_ENR))
                 for k, sp in enumerate(speakers)}
    E = np.stack([embed(enr_paths[k]) for k in range(len(speakers))])
    src_cat = [concat(enr_paths[k], f"src_{l1}_{k}") for k in range(len(speakers))]
    ex_src = extract(src_cat)
    neutral_cat = {v: embed(pool_map(lambda i: render(v, 50, 175, l1, SENTENCES[l1][i], "catn"), range(2))) for v in catalog_variants}
    for k, sp in enumerate(speakers):
        prof = ex_src[k]["profile"]; desc = ex_src[k]["descriptor"]
        f0_src = pyin_median(src_cat[k])
        # oracle_prosody: the default timbre at the speaker's MEASURED pitch and true tempo
        # (the upper bound of any pitch+tempo approach with a fixed voice).
        conds = {"baseline": (DEFAULT_VOICE["v"], DEFAULT_VOICE["p"], DEFAULT_VOICE["s"]),
                 "oracle_prosody": (DEFAULT_VOICE["v"], espeak_p_for(DEFAULT_VOICE["v"], f0_src) if f0_src > 0 else 50, sp["s"]),
                 "oracle_full": (sp["v"], sp["p"], sp["s"])}
        best = max([v for v in catalog_variants if v != sp["v"]], key=lambda v: float(neutral_cat[v] @ E[k]))
        conds["oracle_select"] = (best, espeak_p_for(best, f0_src) if f0_src > 0 else 50, sp["s"])
        if desc is not None:
            p_r, s_r, target = descriptor_to_params(desc, DEFAULT_VOICE["v"])
            conds["vm_prosody"] = (DEFAULT_VOICE["v"], p_r, s_r)
            reg = "low" if target < 145 else ("high" if target > 200 else "mid")
            conds["vm_register"] = (CATALOG[reg], 50, DEFAULT_VOICE["s"] * min(1.2, max(0.8, desc["tempoPct"] / 100)))
        for cond, (v, p, s) in conds.items():
            outs = pool_map(lambda i: render(v, p, s, l2, SENTENCES[l2][N_ENR + i], "out"), range(N_OUT))
            e = embed(outs)
            sims = E @ e
            out_cat = concat(outs, f"o_{l1}{l2}_{k}_{cond}")
            f0_out = pyin_median(out_cat)
            rows.append(dict(l1=l1, l2=l2, spk=k, cond=cond, cos=float(sims[k]), params=(v, float(p), float(s)),
                             top1=int(np.argmax(sims) == k),
                             rank=int((sims > sims[k]).sum()) + 1,
                             pitch_err_st=abs(st(f0_out, f0_src)) if f0_out > 0 and f0_src > 0 else float("nan"),
                             tempo_err_pct=abs(100 * (s / sp["s"] - 1)),
                             had_descriptor=desc is not None))
    imp = [float(E[i] @ E[j]) for i in range(len(E)) for j in range(len(E)) if i != j]
    rows.append(dict(l1=l1, l2=l2, cond="_impostor", cos=float(np.mean(imp))))
    print(f"    {l1}->{l2} done in {time.time() - t_pair:.1f}s")

CONDS = ["baseline", "vm_prosody", "vm_register", "oracle_prosody", "oracle_select", "oracle_full"]
def summarize(rs):
    out = {}
    for c in CONDS:
        r = [x for x in rs if x["cond"] == c]
        if not r: continue
        out[c] = {"n": len(r), "cos": stats([x["cos"] for x in r]), "top1_rate": float(np.mean([x["top1"] for x in r])),
                  "mean_rank": float(np.mean([x["rank"] for x in r])),
                  "pitch_err_st": stats([x["pitch_err_st"] for x in r]), "tempo_err_pct": stats([x["tempo_err_pct"] for x in r])}
    out["_impostor_cos_mean"] = float(np.mean([x["cos"] for x in rs if x["cond"] == "_impostor"]))
    return out
results["identity"] = {
    "n_speakers": len(speakers), "pairs": len(pairs), "enroll_sentences": N_ENR, "output_sentences": N_OUT,
    "descriptor_coverage": float(np.mean([x["had_descriptor"] for x in rows if x.get("cond") == "baseline"])),
    "overall": summarize(rows),
    "by_pair": {f"{a}->{b}": summarize([x for x in rows if x.get("l1") == a and x.get("l2") == b]) for a, b in pairs},
}
results["identity_rows"] = rows

# ============================================================ 4. intelligibility proxy
print("[4] intelligibility proxy (pocketsphinx, English targets only)")
def wer(ref, hyp):
    r, h = ref.lower().split(), hyp.lower().split()
    d = np.zeros((len(r) + 1, len(h) + 1), int); d[:, 0] = range(len(r) + 1); d[0, :] = range(len(h) + 1)
    for i in range(1, len(r) + 1):
        for j in range(1, len(h) + 1):
            d[i, j] = min(d[i - 1, j] + 1, d[i, j - 1] + 1, d[i - 1, j - 1] + (r[i - 1] != h[j - 1]))
    return d[-1, -1] / max(1, len(r))
try:
    from pocketsphinx import Decoder
    dec = Decoder(samprate=16000, loglevel="FATAL")
    def asr(path):
        x, sr = read_wav(path)
        import librosa
        y = librosa.resample(x, orig_sr=sr, target_sr=16000)
        dec.start_utt(); dec.process_raw((np.clip(y, -1, 1) * 32767).astype(np.int16).tobytes(), full_utt=True); dec.end_utt()
        return dec.hyp().hypstr if dec.hyp() else ""
    import re
    norm = lambda t: re.sub(r"[^a-z' ]", " ", t.lower())
    intel = {}
    en_rows = [x for x in rows if x.get("l2") == "en" and x["cond"] in CONDS]
    for c in CONDS:
        ws = []
        for x in [r for r in en_rows if r["cond"] == c][: (12 if QUICK else 40)]:
            v, p, s = x["params"]  # exactly what was rendered in stage 3
            for i in range(N_OUT):
                text = SENTENCES["en"][N_ENR + i]
                ws.append(wer(norm(text), norm(asr(render(v, p, s, "en", text, "out")))))
        if ws: intel[c] = stats(ws)
    # vm conditions: param sweep equivalent — render with the clamped extremes the product can emit
    sweep = {}
    for label, (p, s) in {"pitch_low_clamp": (espeak_p_for("m1", 55 * 2 ** (10 / 12)), 175), "pitch_high_clamp": (espeak_p_for("m1", 55 * 2 ** (34 / 12)), 175),
                          "tempo_0.80": (50, 140), "tempo_1.25": (50, 219), "neutral": (50, 175)}.items():
        ws = [wer(norm(SENTENCES["en"][i]), norm(asr(render("m1", p, s, "en", SENTENCES["en"][i], "intel")))) for i in range(len(SENTENCES["en"]))]
        sweep[label] = stats(ws)
    results["intelligibility_proxy"] = {"by_condition": intel, "param_extremes": sweep,
                                        "note": "pocketsphinx en-us on espeak speech; absolute WER is high for all conditions; compare relatively"}
except Exception as e:  # pragma: no cover
    results["intelligibility_proxy"] = {"error": repr(e)}

# ============================================================ 5. robustness
print("[5] robustness")
base_paths = [concat([render(sp["v"], sp["p"], sp["s"], "en", SENTENCES["en"][i], "rob") for i in range(3)], f"rob_{k}")
              for k, sp in enumerate(speakers)]
clean = extract(base_paths)
variants = {}
def degrade(path, kind):
    x, sr = read_wav(path)
    if kind == "noise_only": x = RNG.normal(0, 0.05, int(3 * sr)).astype(np.float32)
    elif kind.startswith("noise_"):
        snr = float(kind.split("_")[1]); p = np.mean(x ** 2); x = x + RNG.normal(0, math.sqrt(p / 10 ** (snr / 10)), x.shape).astype(np.float32)
    elif kind.startswith("short"):
        secs = float(kind.split("_")[1]); x = x[int(0.2 * sr): int((0.2 + secs) * sr)]
    elif kind == "clipped": x = np.clip(x * 8, -1, 1)
    elif kind == "quiet": x = x * 0.02
    elif kind == "silence": x = np.zeros(int(3 * sr), np.float32)
    elif kind == "resample_48k":
        import librosa; x = librosa.resample(x, orig_sr=sr, target_sr=48000); sr = 48000
    return write_wav(os.path.join(TMP, f"deg_{kind}_{os.path.basename(path)}"), x, sr)
kinds = ["noise_20", "noise_10", "noise_0", "short_0.5", "short_1.0", "short_2.0", "clipped", "quiet", "resample_48k", "silence", "noise_only"]
rob = {}
for kind in kinds:
    ex = extract([degrade(p, kind) for p in base_paths])
    produced = [e["descriptor"] is not None for e in ex]
    perr = [abs(st(e["profile"]["f0MedianHz"], c["profile"]["f0MedianHz"])) for e, c in zip(ex, clean)
            if e["descriptor"] is not None and c["profile"]["f0MedianHz"] > 0]
    same_step = [e["descriptor"]["pitchStep"] == c["descriptor"]["pitchStep"] for e, c in zip(ex, clean) if e["descriptor"] and c["descriptor"]]
    rob[kind] = {"descriptor_rate": float(np.mean(produced)), "pitch_err_vs_clean_st": stats(perr),
                 "same_pitch_step_rate": float(np.mean(same_step)) if same_step else None}
rob["clean_descriptor_rate"] = float(np.mean([c["descriptor"] is not None for c in clean]))
results["robustness"] = rob

# ============================================================ 6. resources
print("[6] resources")
long_path = concat(base_paths[:6], "long")
lx, lsr = read_wav(long_path)
ex_long = extract([long_path])[0]
results["resources"] = {
    "analyzer_ms_per_audio_second_node": ex_long["analyzeMs"] / ex_long["seconds"],
    "audio_seconds": ex_long["seconds"], "sample_rate": lsr,
    "descriptor_json_bytes": len(json.dumps(clean[0]["descriptor"], separators=(",", ":"))) if clean[0]["descriptor"] else None,
    "cpu": os.cpu_count(), "gpu": False,
}
results["meta"]["elapsed_s"] = time.time() - T0
path = os.path.join(OUT, "voicemirror_eval.json")
json.dump(results, open(path, "w"), indent=1)
print(json.dumps({k: results[k] for k in ("calibration",)}, indent=1)[:2000])
print(json.dumps(results["identity"]["overall"], indent=1))
print("wrote", path, f"in {results['meta']['elapsed_s']:.0f}s")
