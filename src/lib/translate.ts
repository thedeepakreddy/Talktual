// Free translation via MyMemory — no signup required, CORS-enabled so it
// can be called directly from the browser (unlike LibreTranslate's public
// instance, which requires a backend).
//
// Free limit: 5,000 chars/day per IP without an email, 50,000/day if you
// pass an email via VITE_MYMEMORY_EMAIL (still no signup — just an
// identifier MyMemory uses to raise your rate limit).

const MYMEMORY_EMAIL = import.meta.env.VITE_MYMEMORY_EMAIL; // optional

export async function translateText(
  text: string,
  fromLang: string,
  toLang: string
): Promise<string> {
  try {
    const params = new URLSearchParams({
      q: text,
      langpair: `${fromLang}|${toLang}`,
    });
    if (MYMEMORY_EMAIL) params.set("de", MYMEMORY_EMAIL);

    const res = await fetch(`https://api.mymemory.translated.net/get?${params}`);
    if (!res.ok) throw new Error(`Translation request failed: ${res.status}`);

    const data = await res.json();
    if (data.responseStatus !== 200) {
      throw new Error(data.responseDetails || "Translation failed");
    }

    return data.responseData.translatedText as string;
  } catch (err) {
    console.error("translateText error:", err);
    // Fail gracefully — return original text rather than crashing the call
    return text;
  }
}
