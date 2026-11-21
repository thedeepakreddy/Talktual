"""Evaluation text corpus — written for this project (no third-party text).

Five everyday sentences per language: the first three are used for speaker
enrollment, the last two as "translated output". Japanese is written in kana
only because espeak-ng 1.51 does not read kanji reliably.
"""

ESPEAK_LANG = {"en": "en-us", "es": "es", "de": "de", "hi": "hi", "ja": "ja", "ru": "ru"}

SENTENCES = {
    "en": [
        "The meeting starts at nine tomorrow morning.",
        "Could you send me the address of the hotel?",
        "I think the train will be late again today.",
        "We should buy some bread and fresh vegetables.",
        "My sister lives near the old harbour.",
    ],
    "es": [
        "Mañana tenemos una reunión muy importante.",
        "¿Dónde está la estación de tren más cercana?",
        "Me gustaría reservar una mesa para dos personas.",
        "El autobús llega cada quince minutos.",
        "Hace mucho calor en la ciudad este verano.",
    ],
    "de": [
        "Morgen früh beginnt die Besprechung um neun Uhr.",
        "Können Sie mir bitte die Adresse schicken?",
        "Ich glaube, der Zug hat heute wieder Verspätung.",
        "Wir sollten Brot und frisches Gemüse kaufen.",
        "Meine Schwester wohnt in der Nähe des Hafens.",
    ],
    "hi": [
        "कल सुबह नौ बजे बैठक शुरू होगी।",
        "क्या आप मुझे होटल का पता भेज सकते हैं?",
        "मुझे लगता है आज फिर ट्रेन देर से आएगी।",
        "हमें रोटी और ताज़ी सब्ज़ियाँ खरीदनी चाहिए।",
        "मेरी बहन पुराने बंदरगाह के पास रहती है।",
    ],
    "ja": [
        "あしたの あさ くじに かいぎが はじまります。",
        "ホテルの じゅうしょを おくって もらえますか。",
        "きょうも でんしゃが おくれると おもいます。",
        "パンと やさいを かいに いきましょう。",
        "わたしの あねは みなとの ちかくに すんでいます。",
    ],
    "ru": [
        "Завтра утром встреча начнётся в девять часов.",
        "Не могли бы вы прислать мне адрес гостиницы?",
        "Я думаю, поезд сегодня снова опоздает.",
        "Нам нужно купить хлеб и свежие овощи.",
        "Моя сестра живёт рядом со старым портом.",
    ],
}
