"""M3.1 feasibility probe (synthetic voices only).

Question: for a speaker-encoder notion of "same voice", how much comes from
pitch + tempo (preservable without cloning) vs timbre (needs cloning/conversion)?

Synthetic speakers = espeak-ng variant (timbre) x pitch x speed.
Each speaker "enrolls" with English sentences; output is rendered in Spanish as:
  baseline  - one fixed default voice (what Talktual does today)
  prosody   - default timbre, the speaker's TRUE pitch+speed (oracle for a
              pitch/tempo-matching approach)
  full      - the speaker's own timbre+pitch+speed (oracle for perfect cloning)
Metric: resemblyzer (GE2E) cosine to the speaker's enrollment, and top-1
identification among all speakers.
"""
import itertools, json, os, subprocess, sys, tempfile
import numpy as np
from resemblyzer import VoiceEncoder, preprocess_wav

OUT = sys.argv[1] if len(sys.argv) > 1 else "probe.json"
tmp = tempfile.mkdtemp()
enc = VoiceEncoder("cpu", verbose=False)

def synth(voice, pitch, speed, text, name):
    p = os.path.join(tmp, name + ".wav")
    subprocess.run(["espeak-ng", "-v", voice, "-p", str(pitch), "-s", str(speed), "-w", p, text], check=True)
    return preprocess_wav(p)

EN = ["The meeting starts at nine tomorrow morning.", "Could you send me the address of the hotel?",
      "I think the train will be late again today.", "We should buy some bread and fresh vegetables."]
ES = ["Mañana tenemos una reunión muy importante.", "¿Dónde está la estación de tren más cercana?",
      "Me gustaría reservar una mesa para dos personas."]

variants = ["m1", "m3", "m7", "f1", "f3", "f5"]
speakers = [dict(v=v, p=p, s=s) for v, (p, s) in itertools.product(variants, [(30, 140), (70, 190)])]
DEFAULT = dict(v="m1", p=50, s=175)

def embed(sp, lang, texts, tag):
    wavs = [synth(f"{lang}+{sp['v']}", sp["p"], sp["s"], t, f"{tag}{i}") for i, t in enumerate(texts)]
    return enc.embed_speaker(wavs)

enroll = [embed(sp, "en", EN, f"enr{k}_") for k, sp in enumerate(speakers)]
E = np.stack(enroll)
res = {"baseline": [], "prosody": [], "select": [], "full": []}
# Catalog of stock timbres at neutral prosody (what a provider voice library offers).
catalog = {v: embed(dict(v=v, p=50, s=175), "en", EN[:2], f"cat{v}_") for v in variants}
ident = {k: 0 for k in res}
base_emb = embed(DEFAULT, "es", ES, "base_")
for k, sp in enumerate(speakers):
    # Voice selection: nearest stock timbre from a catalog that does NOT contain
    # this speaker's own timbre (leave-one-out), chosen by embedding similarity
    # (best case for selection), rendered with the speaker's pitch + speed.
    cand = [v for v in variants if v != sp["v"]]
    cand_emb = {v: catalog[v] for v in cand}
    best = max(cand, key=lambda v: float(cand_emb[v] @ E[k]))
    outs = {
        "select": embed(dict(v=best, p=sp["p"], s=sp["s"]), "es", ES, f"sel{k}_"),
        "baseline": base_emb,
        "prosody": embed(dict(v=DEFAULT["v"], p=sp["p"], s=sp["s"]), "es", ES, f"pro{k}_"),
        "full": embed(sp, "es", ES, f"full{k}_"),
    }
    for cond, e in outs.items():
        sims = E @ e
        res[cond].append(float(sims[k]))
        ident[cond] += int(np.argmax(sims) == k)
imp = [float(E[i] @ E[j]) for i in range(len(E)) for j in range(len(E)) if i != j]
summary = {
    "n_speakers": len(speakers),
    "impostor_cosine_mean": float(np.mean(imp)),
    **{f"{c}_cosine_mean": float(np.mean(v)) for c, v in res.items()},
    **{f"{c}_top1_ident": f"{ident[c]}/{len(speakers)}" for c in res},
}
json.dump({"summary": summary, "per_speaker": res, "speakers": speakers}, open(OUT, "w"), indent=1)
print(json.dumps(summary, indent=1))
