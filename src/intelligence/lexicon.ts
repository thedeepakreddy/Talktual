// Closed, reviewable word lists for ACI. Everything ACI "knows" is here: no
// model, no provider. Coverage is deliberately small and documented; words not
// listed get no special handling (faithful translation).

import type { AciLang, SignalCategory } from './types';

export interface Sense {
  id: string;
  /** Keywords describing the sense (UI notes + evaluation matching). */
  gloss: string;
  /** Context words that support this sense (lowercase, matched as whole words). */
  cues: string[];
  /**
   * Unambiguous replacement for MT input when context selects this sense
   * (meaning-preserving synonym). Absent → no pre-edit; a note explains instead.
   */
  edit?: string;
}

export interface AmbiguousWord {
  word: string;
  /** Inflected/plural surface forms that map to this entry. */
  forms?: string[];
  senses: Sense[];
}

const A = (word: string, senses: Sense[], forms?: string[]): AmbiguousWord => ({ word, senses, forms });

export const AMBIGUOUS: Partial<Record<AciLang, AmbiguousWord[]>> = {
  en: [
    A('bank', [
      { id: 'finance', gloss: 'financial institution money bank', cues: ['money', 'account', 'loan', 'deposit', 'cash', 'atm', 'withdraw', 'card', 'transfer', 'cheque', 'check', 'branch', 'savings', 'mortgage', 'teller', 'euros', 'dollars'] },
      { id: 'river', gloss: 'river side riverbank shore', cues: ['river', 'water', 'fishing', 'fish', 'boat', 'swim', 'shore', 'stream', 'canal', 'picnic', 'ducks'], edit: 'riverbank' },
    ], ['banks']),
    A('bat', [
      { id: 'animal', gloss: 'flying animal mammal', cues: ['cave', 'night', 'wings', 'flying', 'fly', 'animal', 'vampire', 'zoo'] },
      { id: 'sport', gloss: 'baseball cricket bat club', cues: ['baseball', 'cricket', 'ball', 'swing', 'hit', 'game', 'team', 'pitch'], edit: 'baseball bat' },
    ], ['bats']),
    A('date', [
      { id: 'calendar', gloss: 'calendar day', cues: ['calendar', 'day', 'month', 'schedule', 'deadline', 'when', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday', 'booking', 'reservation', 'flight', 'meeting'] },
      { id: 'romantic', gloss: 'romantic outing appointment with partner', cues: ['dinner', 'romantic', 'girlfriend', 'boyfriend', 'love', 'first', 'tinder', 'cute', 'kiss'], edit: 'romantic date' },
      { id: 'fruit', gloss: 'fruit palm', cues: ['fruit', 'eat', 'sweet', 'dried', 'palm', 'figs', 'snack'], edit: 'date fruit' },
    ], ['dates']),
    A('glasses', [
      { id: 'spectacles', gloss: 'eyeglasses spectacles see', cues: ['see', 'read', 'reading', 'eyes', 'sight', 'optician', 'lens', 'lenses', 'wear', 'blurry'], edit: 'eyeglasses' },
      { id: 'drinkware', gloss: 'drinking glasses cups', cues: ['wine', 'water', 'drink', 'drinks', 'beer', 'table', 'kitchen', 'pour', 'champagne', 'toast'] },
    ]),
    A('match', [
      { id: 'game', gloss: 'game sports match', cues: ['football', 'soccer', 'tennis', 'team', 'score', 'win', 'won', 'lost', 'stadium', 'tickets', 'goal', 'league'] },
      { id: 'fire', gloss: 'matchstick fire', cues: ['fire', 'light', 'candle', 'cigarette', 'box', 'lighter', 'stove', 'strike'], edit: 'matchstick' },
    ], ['matches']),
    A('ring', [
      { id: 'jewelry', gloss: 'jewelry finger ring', cues: ['finger', 'wedding', 'gold', 'diamond', 'engagement', 'jewelry', 'wear', 'silver'] },
      { id: 'call', gloss: 'phone call', cues: ['phone', 'call', 'tomorrow', 'later', 'number', 'tonight'], edit: 'call' },
    ]),
    A('change', [
      { id: 'coins', gloss: 'coins money returned', cues: ['coins', 'cash', 'pay', 'paid', 'euros', 'dollars', 'note', 'bill', 'price', 'tip', 'register', 'keep'] },
      { id: 'alter', gloss: 'modification difference', cues: ['plan', 'plans', 'mind', 'schedule', 'time', 'date', 'booking', 'flight', 'room'] },
    ]),
    A('fair', [
      { id: 'just', gloss: 'just equitable', cues: ['unfair', 'price', 'deal', 'rules', 'honest', 'share', 'treat', 'not'] },
      { id: 'event', gloss: 'fair event exhibition funfair', cues: ['trade', 'rides', 'stalls', 'visit', 'booth', 'exhibition', 'tickets', 'went'], edit: 'trade fair' },
    ]),
    A('trip', [
      { id: 'journey', gloss: 'journey travel', cues: ['travel', 'holiday', 'vacation', 'flight', 'plan', 'week', 'beach', 'business', 'road'] },
      { id: 'stumble', gloss: 'stumble fall', cues: ['fall', 'fell', 'careful', 'stairs', 'step', 'floor', 'cable', 'over'], edit: 'stumble' },
    ]),
    A('mouse', [
      { id: 'animal', gloss: 'rodent animal', cues: ['cat', 'cheese', 'kitchen', 'trap', 'animal', 'squeak', 'pest'] },
      { id: 'computer', gloss: 'computer mouse device', cues: ['computer', 'laptop', 'click', 'keyboard', 'screen', 'usb', 'wireless', 'pc'], edit: 'computer mouse' },
    ]),
    A('spring', [
      { id: 'season', gloss: 'season springtime', cues: ['summer', 'winter', 'autumn', 'season', 'april', 'march', 'may', 'flowers', 'weather', 'next'], edit: 'springtime' },
      { id: 'water', gloss: 'water source', cues: ['water', 'hot', 'mountain', 'drink', 'thermal', 'source'], edit: 'water spring' },
      { id: 'coil', gloss: 'metal coil', cues: ['mattress', 'broken', 'metal', 'coil', 'seat', 'bed'], edit: 'metal spring' },
    ]),
    A('letter', [
      { id: 'mail', gloss: 'written message mail', cues: ['send', 'sent', 'post', 'mail', 'envelope', 'stamp', 'wrote', 'write', 'received', 'reply'] },
      { id: 'alphabet', gloss: 'alphabet character', cues: ['alphabet', 'spell', 'spelled', 'capital', 'word', 'name', 'starts', 'begins', 'first'] },
    ], ['letters']),
    A('fan', [
      { id: 'device', gloss: 'cooling device', cues: ['hot', 'heat', 'air', 'turn', 'ceiling', 'room', 'cool', 'electric'], edit: 'electric fan' },
      { id: 'supporter', gloss: 'supporter enthusiast', cues: ['team', 'big', 'music', 'band', 'football', 'club', 'concert', 'huge'] },
    ], ['fans']),
    A('right', [
      { id: 'direction', gloss: 'direction right side', cues: ['turn', 'left', 'street', 'corner', 'side', 'door', 'go', 'take', 'next'] },
      { id: 'correct', gloss: 'correct true', cues: ['answer', 'wrong', 'true', 'correct', "you're", 'exactly', 'absolutely'] },
    ]),
  ],
  es: [
    A('banco', [
      { id: 'finance', gloss: 'financial institution money bank', cues: ['dinero', 'cuenta', 'préstamo', 'tarjeta', 'cajero', 'transferencia', 'sacar', 'ingresar', 'euros', 'sucursal', 'hipoteca'] },
      { id: 'bench', gloss: 'bench seat', cues: ['sentar', 'sentarnos', 'sentado', 'sentada', 'sentamos', 'senté', 'parque', 'plaza', 'madera', 'descansar', 'jardín'], edit: 'asiento' },
    ], ['bancos']),
    A('vela', [
      { id: 'candle', gloss: 'candle wax', cues: ['luz', 'cumpleaños', 'encender', 'apagar', 'cera', 'tarta', 'apagón', 'mesa'] },
      { id: 'sail', gloss: 'sail boat sailing', cues: ['barco', 'viento', 'mar', 'navegar', 'regata', 'velero', 'puerto'] },
    ], ['velas']),
    A('carta', [
      { id: 'letter', gloss: 'letter mail written message', cues: ['enviar', 'envié', 'correo', 'sobre', 'sello', 'escribir', 'escribí', 'recibí', 'buzón'] },
      { id: 'menu', gloss: 'restaurant menu', cues: ['restaurante', 'camarero', 'pedir', 'comer', 'cenar', 'plato', 'postre', 'vino', 'mesa'] },
      { id: 'card', gloss: 'playing card', cues: ['jugar', 'baraja', 'póker', 'partida', 'cartas', 'as'] },
    ], ['cartas']),
    A('cita', [
      { id: 'appointment', gloss: 'appointment meeting scheduled', cues: ['médico', 'dentista', 'hora', 'consulta', 'oficina', 'reunión', 'trámite', 'pedir', 'mañana', 'lunes', 'martes', 'miércoles', 'jueves', 'viernes'] },
      { id: 'romantic', gloss: 'romantic date outing with partner', cues: ['novio', 'novia', 'romántica', 'primera', 'cenar', 'amor', 'pareja'] },
      { id: 'quote', gloss: 'quotation quote text', cues: ['libro', 'autor', 'frase', 'famosa', 'texto', 'leí'] },
    ], ['citas']),
    A('muñeca', [
      { id: 'doll', gloss: 'toy doll', cues: ['juguete', 'niña', 'regalo', 'jugar', 'tienda'] },
      { id: 'wrist', gloss: 'wrist joint', cues: ['duele', 'dolor', 'mano', 'brazo', 'reloj', 'rota', 'caí', 'torcí'] },
    ]),
    A('planta', [
      { id: 'plant', gloss: 'plant vegetation', cues: ['regar', 'agua', 'flores', 'maceta', 'hojas', 'jardín', 'verde'] },
      { id: 'floor', gloss: 'building floor storey level', cues: ['ascensor', 'edificio', 'piso', 'escalera', 'primera', 'segunda', 'tercera', 'baja', 'subir'] },
    ], ['plantas']),
    A('cola', [
      { id: 'queue', gloss: 'queue line waiting', cues: ['esperar', 'gente', 'fila', 'entrada', 'larga', 'taquilla', 'caja'] },
      { id: 'tail', gloss: 'animal tail', cues: ['perro', 'gato', 'animal', 'mueve', 'pelo'] },
      { id: 'soda', gloss: 'cola soft drink soda', cues: ['beber', 'refresco', 'vaso', 'hielo', 'botella', 'lata'] },
    ]),
    A('tienda', [
      { id: 'shop', gloss: 'shop store', cues: ['comprar', 'compré', 'abierta', 'cerrada', 'precio', 'ropa', 'calle', 'centro'] },
      { id: 'tent', gloss: 'tent camping', cues: ['camping', 'acampar', 'montar', 'monte', 'saco', 'dormir', 'montaña'] },
    ], ['tiendas']),
    A('hoja', [
      { id: 'leaf', gloss: 'plant leaf tree', cues: ['árbol', 'otoño', 'verde', 'caen', 'planta'] },
      { id: 'sheet', gloss: 'sheet of paper page form', cues: ['papel', 'firmar', 'escribir', 'imprimir', 'formulario', 'rellenar'] },
    ], ['hojas']),
    A('gato', [
      { id: 'cat', gloss: 'cat animal pet', cues: ['perro', 'mascota', 'animal', 'maúlla', 'comida', 'veterinario'] },
      { id: 'jack', gloss: 'car jack tool', cues: ['rueda', 'coche', 'pinchazo', 'maletero', 'neumático', 'levantar'] },
    ]),
  ],
};

export interface Idiom {
  phrase: string;
  meaning: string;
}
export const IDIOMS: Partial<Record<AciLang, Idiom[]>> = {
  en: [
    { phrase: 'break a leg', meaning: 'good luck' },
    { phrase: 'piece of cake', meaning: 'very easy' },
    { phrase: 'under the weather', meaning: 'slightly unwell' },
    { phrase: 'call it a day', meaning: 'stop working for today' },
    { phrase: 'hit the road', meaning: 'leave / set off' },
    { phrase: 'on the same page', meaning: 'in agreement' },
    { phrase: 'cost an arm and a leg', meaning: 'very expensive' },
    { phrase: 'costs an arm and a leg', meaning: 'very expensive' },
    { phrase: 'once in a blue moon', meaning: 'very rarely' },
    { phrase: 'beat around the bush', meaning: 'avoid the main point' },
    { phrase: 'let the cat out of the bag', meaning: 'reveal a secret' },
    { phrase: 'a piece of cake', meaning: 'very easy' },
    { phrase: 'hang in there', meaning: "don't give up" },
    { phrase: 'no big deal', meaning: 'not important' },
    { phrase: 'keep an eye on', meaning: 'watch / look after' },
    { phrase: 'it is raining cats and dogs', meaning: 'raining very heavily' },
    { phrase: "it's raining cats and dogs", meaning: 'raining very heavily' },
    { phrase: 'raining cats and dogs', meaning: 'raining very heavily' },
    { phrase: 'pull my leg', meaning: 'joke with me' },
    { phrase: 'pulling my leg', meaning: 'joking with me' },
    { phrase: 'out of the blue', meaning: 'unexpectedly' },
    { phrase: 'sleep on it', meaning: 'decide later after thinking' },
  ],
  es: [
    { phrase: 'tomar el pelo', meaning: 'to joke with / tease someone' },
    { phrase: 'tomando el pelo', meaning: 'joking with / teasing someone' },
    { phrase: 'estar en las nubes', meaning: 'daydreaming, distracted' },
    { phrase: 'en las nubes', meaning: 'daydreaming, distracted' },
    { phrase: 'costar un ojo de la cara', meaning: 'very expensive' },
    { phrase: 'cuesta un ojo de la cara', meaning: 'very expensive' },
    { phrase: 'un ojo de la cara', meaning: 'very expensive' },
    { phrase: 'meter la pata', meaning: 'make a blunder' },
    { phrase: 'metí la pata', meaning: 'I made a blunder' },
    { phrase: 'no tener pelos en la lengua', meaning: 'speak bluntly' },
    { phrase: 'pan comido', meaning: 'very easy' },
    { phrase: 'estar como una cabra', meaning: 'be a bit crazy' },
    { phrase: 'como una cabra', meaning: 'a bit crazy' },
    { phrase: 'dar la lata', meaning: 'to pester' },
    { phrase: 'ponerse las pilas', meaning: 'get going, get serious' },
    { phrase: 'ponte las pilas', meaning: 'get going, get serious' },
    { phrase: 'echar una mano', meaning: 'to help' },
    { phrase: 'me echas una mano', meaning: 'can you help me' },
    { phrase: 'quedarse en blanco', meaning: 'mind going blank' },
    { phrase: 'me quedé en blanco', meaning: 'my mind went blank' },
    { phrase: 'a otra cosa mariposa', meaning: "let's move on" },
    { phrase: 'ser pan comido', meaning: 'very easy' },
  ],
};

/** Signal cue patterns. Each has a fixed confidence (rule strength). Tested as regexes on lowercased text. */
export interface Cue {
  re: RegExp;
  confidence: number;
}
type CueTable = Partial<Record<SignalCategory, Cue[]>>;
const c = (re: RegExp, confidence: number): Cue => ({ re, confidence });

export const SIGNAL_CUES: Partial<Record<AciLang, CueTable>> = {
  en: {
    question: [
      c(/\?\s*$/, 0.9),
      c(/^(what|where|when|who|whom|whose|why|how|which)\b/, 0.8),
      c(/^(do|does|did|is|are|was|were|can|could|would|will|should|shall|have|has|may|am)\s+(i|you|we|they|he|she|it|there|this|that)\b/, 0.75),
    ],
    request: [
      c(/\bplease\b/, 0.75),
      c(/^(can|could|would|will)\s+you\b/, 0.8),
      c(/\b(i need you to|i'd like you to|would you mind|do me a favou?r)\b/, 0.85),
      c(/(^|,\s*)(please\s+)?(call|send|bring|take|wait|come|give|tell|show|help|stop|open|close|book|check|meet|pick|grab|pass|turn|hold|let me know|remind|text|email|write|sit|look|follow|follow me)\b/, 0.65),
    ],
    offer: [
      c(/\b(shall i|should i|do you want me to|want me to|would you like me to|let me|i can|i could|i'll|i will)\s+(help|take|bring|get|show|call|drive|pick|carry|book|pay|send|walk|give|lend|buy|check|make|grab|come|meet|cook|order)\b/, 0.8),
      c(/\b(would you like|do you want|how about|what about|why don't (we|i))\b/, 0.6),
      c(/\b(will|can|could)\s+(pick you up|take you|show you|drive you|help you|bring you|meet you|walk you)\b/, 0.65),
    ],
    confirmation: [
      c(/^(yes|yeah|yep|yup|sure|ok|okay|alright|all right|sounds good|that works|works for me|exactly|correct|perfect|deal|of course|agreed|definitely|absolutely|great|fine|got it)\b/, 0.8),
      c(/\b(that works|sounds good|see you then|it's a deal|i agree|that's right|you're right)\b/, 0.7),
    ],
    refusal: [
      c(/^(no|nope|nah)\b(?!,?\s*(i meant|i mean|not\b|it's|it is|i said|sorry))/, 0.75),
      c(/\b(i can't|i cannot|i won't|i will not|not possible|i don't want|i'd rather not|no way|no thanks|no thank you|i'm not going to|that won't work|that doesn't work for me)\b/, 0.8),
    ],
    correction: [
      c(/\b(i meant|i mean,|what i meant|sorry,? i (said|meant)|i said .* but|correction|let me correct|i got that wrong|i was wrong|my mistake)\b/, 0.85),
      c(/^(no|sorry|wait|actually),?\s+(not\b|it's|it is|it was|i said|i meant|i mean|make that|rather)/, 0.85),
      c(/\bnot\s+\w+,?\s+(but|it's)\b/, 0.55),
      c(/\bmake (that|it) (at |for )?(\d|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|monday|tuesday|wednesday|thursday|friday|saturday|sunday|tomorrow)/, 0.7),
    ],
    'clarification-request': [
      c(/^(sorry|pardon|what|huh|excuse me)\s*\??\s*$/, 0.9),
      c(/\b(come again|say that again|can you repeat|could you repeat|repeat that|what do you mean|what did you say|i didn't (get|catch|understand|hear)|could you explain|can you explain|what does .* mean|did you say)\b/, 0.85),
    ],
    urgency: [c(/\b(hurry|urgent|urgently|emergency|right now|immediately|asap|as soon as possible|quickly|right away|help me|call an ambulance|call the police)\b/, 0.85)],
    'topic-change': [c(/^(by the way|anyway|changing the subject|on another note|speaking of|another thing|btw|oh,? and|so,? about|also,? about|different question|on a different note)\b/, 0.8)],
  },
  es: {
    question: [
      c(/\?\s*$/, 0.9),
      c(/^¿/, 0.9),
      c(/^(qué|que|dónde|donde|cuándo|cuando|quién|quien|por qué|porqué|cómo|como|cuál|cual|cuánto|cuanto|cuánta|cuántos|cuántas)\b.*\?/, 0.85),
      c(/^(qué|dónde|cuándo|quién|por qué|cómo|cuál|cuánto|cuánta|cuántos|cuántas)\b/, 0.75),
    ],
    request: [
      c(/\bpor favor\b/, 0.75),
      c(/^(¿\s*)?(puedes|podrías|podéis|podría|me puedes|me podrías|me das|me pasas|me traes|me dejas|me ayudas|me echas)\b/, 0.8),
      c(/(^|,\s*)(llama|llámame|manda|mándame|envía|envíame|espera|espérame|ven|dime|trae|tráeme|dame|ayúdame|cierra|abre|mira|reserva|escribe|escríbeme|avísame|pásame|para|pon|siéntate|sígueme)\b/, 0.7),
      c(/\b(necesito que|quiero que (me|tú|vengas|llames|traigas))\b/, 0.75),
    ],
    offer: [
      c(/\b(te llevo|te ayudo|te acompaño|te invito|te traigo|te llamo|te paso|te enseño|te recojo|te presto|puedo (ayudarte|llevarte|llamar|traer|pagar|reservar))\b/, 0.8),
      c(/\b(quieres que|si quieres|te apetece|¿te parece|qué tal si)\b/, 0.6),
    ],
    confirmation: [
      c(/^(sí|si,|vale|claro|de acuerdo|perfecto|exacto|exactamente|eso es|genial|por supuesto|venga|bueno,? vale|muy bien|entendido|okey|ok)\b/, 0.8),
      c(/\b(me parece bien|está bien|nos vemos entonces|trato hecho|tienes razón)\b/, 0.7),
    ],
    refusal: [
      c(/^no\b(?!,?\s*(quise|quería|perdón|digo|es|era|a las|el|la)\b)/, 0.7),
      c(/\b(no puedo|no quiero|imposible|ni hablar|mejor no|no gracias|no, gracias|de ninguna manera|no me viene bien|no me va bien)\b/, 0.8),
    ],
    correction: [
      c(/\b(quise decir|quería decir|me equivoqué|me he equivocado|perdón,? (no|digo|era|quise)|mejor dicho|digo,|o sea,? no|corrijo)\b/, 0.85),
      c(/^(no|perdón|espera|perdona),?\s+(no\s+(es|era|a las|el|la|en)\b|es\b|era\b|digo\b|a las\b|el\b|la\b|en\b)/, 0.75),
      c(/\bno\s+\w+,?\s+(sino|es)\b/, 0.55),
    ],
    'clarification-request': [
      c(/^¿?\s*(cómo|qué|perdona|perdón|eh|mande)\s*\??\s*$/, 0.9),
      c(/\b(no (te )?entiendo|no te he entendido|no he entendido|puedes repetir|me lo repites|repite|repítelo|qué quieres decir|cómo dices|qué has dicho|no te oigo|más despacio|qué significa)\b/, 0.85),
    ],
    urgency: [c(/\b(urgente|urgencia|emergencia|ahora mismo|rápido|deprisa|inmediatamente|cuanto antes|socorro|ya mismo|date prisa|corre|llama a una ambulancia|llama a la policía)\b/, 0.85)],
    'topic-change': [c(/^(por cierto|cambiando de tema|otra cosa|a propósito|oye,? (por cierto|otra cosa)|hablando de|una pregunta diferente|bueno,? otra cosa)\b/, 0.8)],
  },
};

/** Explicit non-understanding (feeds `misunderstanding`). */
export const NOT_UNDERSTOOD: Partial<Record<AciLang, RegExp>> = {
  en: /\b(i don't understand|i do not understand|i didn't understand|that doesn't make sense|that makes no sense|i'm confused|i am confused|i'm lost|what do you mean)\b/,
  es: /\b(no entiendo|no te entiendo|no he entendido|no tiene sentido|estoy confundid[oa]|me he perdido|qué quieres decir)\b/,
};

/** Discourse fillers removable in concise mode (closed list; never content words). */
export const FILLERS: Partial<Record<AciLang, string[]>> = {
  en: ['you know', 'i mean', 'basically', 'um', 'uh', 'erm', 'like,', 'well,', 'so,'],
  es: ['o sea', 'pues', 'bueno,', 'este,', 'eh', 'em', 'digamos,'],
  fr: ['euh', 'ben', 'bon,', 'tu vois', 'quoi,'],
  de: ['äh', 'ähm', 'also,', 'halt', 'naja,'],
  pt: ['tipo,', 'né', 'então,', 'hum'],
  it: ['cioè', 'ehm', 'allora,', 'tipo,', 'insomma,'],
};

/** Reference words and the entity kinds they can point to. */
export const REFERENCES: Partial<Record<AciLang, Record<string, 'place' | 'person'>>> = {
  en: { there: 'place', he: 'person', she: 'person', him: 'person', her: 'person' },
  es: { allí: 'place', allá: 'place', ahí: 'place', él: 'person', ella: 'person' },
};

/** Words that introduce places / persons (entity-kind cues). */
export const PLACE_PREP: Partial<Record<AciLang, string[]>> = {
  en: ['in', 'at', 'to', 'from', 'near', 'into', 'outside', 'inside', 'towards', 'opposite'],
  es: ['en', 'a', 'al', 'de', 'del', 'desde', 'hacia', 'hasta', 'cerca de', 'junto a', 'enfrente de'],
};
export const PLACE_WORDS = new Set(
  'street st road avenue ave square plaza station airport hotel museum park bridge market cathedral church beach mall center centre restaurant cafe café bar hospital university airport terminal gate theatre theater cinema gallery library stadium club pub school office tower palace castle zoo arena hall calle teatro cine galería biblioteca estadio palacio castillo torre colegio oficina avenida plaza paseo estación aeropuerto museo parque puente mercado catedral iglesia playa hospital universidad hotel restaurante barrio carretera'.split(
    ' '
  )
);
export const ADDRESS_WORDS = new Set('street st road avenue ave square calle avenida plaza paseo carretera camino'.split(' '));
export const PERSON_CUES = new Set('mr mrs ms miss dr doctor señor señora sr sra srta don doña'.split(' '));
export const PERSON_INTRO: Partial<Record<AciLang, RegExp>> = {
  en: /\b(my (friend|colleague|boss|brother|sister|wife|husband|partner|cousin|neighbou?r|son|daughter|mother|father|mom|dad|uncle|aunt)|called|named|ask|tell|meet|met|with|from|for|is|was)\s*$/,
  es: /\b(mi (amig[oa]|compañer[oa]|jef[ae]|herman[oa]|mujer|marido|pareja|prim[oa]|vecin[oa]|hij[oa]|madre|padre|tí[oa])|llamad[oa]|se llama|pregúntale a|dile a|con|para|a)\s*$/,
};

export const DAYS: Partial<Record<AciLang, string[]>> = {
  en: ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday', 'today', 'tomorrow', 'tonight', 'yesterday'],
  es: ['lunes', 'martes', 'miércoles', 'jueves', 'viernes', 'sábado', 'domingo', 'hoy', 'mañana', 'ayer', 'pasado mañana', 'esta noche'],
};
export const MONTHS: Partial<Record<AciLang, string[]>> = {
  en: ['january', 'february', 'march', 'april', 'june', 'july', 'august', 'september', 'october', 'november', 'december'],
  es: ['enero', 'febrero', 'marzo', 'abril', 'mayo', 'junio', 'julio', 'agosto', 'septiembre', 'octubre', 'noviembre', 'diciembre'],
};
export const NUMBER_WORDS: Partial<Record<AciLang, string[]>> = {
  en: ['one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine', 'ten', 'eleven', 'twelve', 'fifteen', 'twenty', 'thirty', 'half'],
  es: ['una', 'uno', 'dos', 'tres', 'cuatro', 'cinco', 'seis', 'siete', 'ocho', 'nueve', 'diez', 'once', 'doce', 'quince', 'veinte', 'treinta', 'media'],
};

/** Very common words (topic extraction ignores them). */
export const STOP = new Set(
  `the a an and or but if so to of in on at for from by with about as is are was were be been being am i you he she it we they me him her us them my your his its our their this that these those there here what where when who why how which do does did done have has had will would can could should shall may might must not no yes ok okay please just very really too also then than now well oh yeah sure get got go going come let lets let's want need like know think see say said tell one some any all more most much many lot thing things time way good great fine right i'm you're it's that's don't can't won't i'll we'll there's
  el la los las un una unos unas y o pero si no sí que de del al a en con por para sobre como es son era fue ser estar está están estoy estás muy más menos ya también entonces bueno vale pues yo tú él ella nosotros vosotros ellos ellas me te se nos os le les lo mi tu su mis tus sus este esta esto ese esa eso aquí allí hay tengo tienes tiene vamos voy vas va quiero quieres puedo puedes hacer dicho decir cosa cosas tiempo`.split(/\s+/)
);
