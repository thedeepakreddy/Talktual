// Free translation via LibreTranslate's public instance.
// Swap this file's internals later for DeepL; keep the same function
// signature (text, from, to -> translated string) so nothing else changes.

export async function translateText(
  text: string,
  fromLang: string,
  toLang: string
): Promise<string> {
  try {
    const res = await fetch("https://libretranslate.com/translate", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        q: text,
        source: fromLang,
        target: toLang,
        format: "text",
      }),
    });

    if (!res.ok) {
      throw new Error(`Translation request failed: ${res.status}`);
    }

    const data = await res.json();
    return data.translatedText as string;
  } catch (err) {
    console.error("translateText error:", err);
    // Fail gracefully — return original text rather than crashing the call
    return text;
  }
}
