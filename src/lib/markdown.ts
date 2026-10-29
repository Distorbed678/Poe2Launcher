/**
 * A small, safe markdown subset for news, release notes and a server's message of the day.
 * The result is a tree of plain nodes that the UI renders with normal Svelte text bindings,
 * so no HTML in the source is ever interpreted: `<script>` is just text. Links must be https.
 *
 * Supported: `#`/`##`/`###` headings, `-`/`*`/`+` and `1.` lists, `>` quotes, fenced code,
 * paragraphs, `**bold**`, `*italic*`/`_italic_`, `` `code` `` and `[text](https://...)`.
 */

export type Inline =
  | { t: 'text'; s: string }
  | { t: 'b'; c: Inline[] }
  | { t: 'i'; c: Inline[] }
  | { t: 'code'; s: string }
  | { t: 'a'; href: string; c: Inline[] };

export type Block =
  | { t: 'p'; c: Inline[] }
  | { t: 'h'; level: 1 | 2 | 3; c: Inline[] }
  | { t: 'ul'; items: Inline[][] }
  | { t: 'ol'; items: Inline[][] }
  | { t: 'quote'; c: Inline[] }
  | { t: 'code'; s: string };

/** Only these links are made clickable. */
export function safeHref(url: string): string | null {
  const u = url.trim();
  if (!/^https:\/\/[^\s<>"'`]+$/i.test(u) || u.length > 300) return null;
  return u;
}

const MAX_DEPTH = 3;

/** Inline formatting of one paragraph's text. */
export function parseInline(src: string, depth = 0): Inline[] {
  const out: Inline[] = [];
  let text = '';
  const flush = () => {
    if (text) out.push({ t: 'text', s: text });
    text = '';
  };
  let i = 0;
  while (i < src.length) {
    const ch = src[i]!;
    // Escapes: \* \_ \` \[ \\
    if (ch === '\\' && i + 1 < src.length && /[\\`*_[\]()#>-]/.test(src[i + 1]!)) {
      text += src[i + 1];
      i += 2;
      continue;
    }
    if (ch === '`') {
      const end = src.indexOf('`', i + 1);
      if (end > i + 1) {
        flush();
        out.push({ t: 'code', s: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if (depth < MAX_DEPTH && (src.startsWith('**', i) || src.startsWith('__', i))) {
      const mark = src.slice(i, i + 2);
      const end = src.indexOf(mark, i + 2);
      if (end > i + 2) {
        flush();
        out.push({ t: 'b', c: parseInline(src.slice(i + 2, end), depth + 1) });
        i = end + 2;
        continue;
      }
    }
    if (depth < MAX_DEPTH && (ch === '*' || ch === '_') && !/\s/.test(src[i + 1] ?? ' ')) {
      // `_` inside a word (snake_case) is not emphasis.
      const prev = src[i - 1] ?? ' ';
      if (!(ch === '_' && /\w/.test(prev))) {
        const end = src.indexOf(ch, i + 1);
        if (end > i + 1 && !/\s/.test(src[end - 1]!) && !(ch === '_' && /\w/.test(src[end + 1] ?? ' '))) {
          flush();
          out.push({ t: 'i', c: parseInline(src.slice(i + 1, end), depth + 1) });
          i = end + 1;
          continue;
        }
      }
    }
    if (ch === '[' && depth < MAX_DEPTH) {
      const close = src.indexOf('](', i + 1);
      const end = close > 0 ? src.indexOf(')', close + 2) : -1;
      if (close > i && end > close) {
        const href = safeHref(src.slice(close + 2, end));
        if (href) {
          flush();
          out.push({ t: 'a', href, c: parseInline(src.slice(i + 1, close), depth + 1) });
          i = end + 1;
          continue;
        }
        // An unsafe link shows its text only.
        text += src.slice(i + 1, close);
        i = end + 1;
        continue;
      }
    }
    text += ch;
    i++;
  }
  flush();
  return out;
}

const BULLET = /^\s{0,3}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,3}\d{1,3}[.)]\s+(.*)$/;
const HEADING = /^\s{0,3}(#{1,3})\s+(.*?)\s*#*\s*$/;

export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const blocks: Block[] = [];
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) blocks.push({ t: 'p', c: parseInline(para.join(' ')) });
    para = [];
  };
  for (let n = 0; n < lines.length; n++) {
    const line = lines[n]!;
    if (/^\s*```/.test(line)) {
      flushPara();
      const code: string[] = [];
      n++;
      while (n < lines.length && !/^\s*```/.test(lines[n]!)) code.push(lines[n++]!);
      blocks.push({ t: 'code', s: code.join('\n') });
      continue;
    }
    if (line.trim() === '') {
      flushPara();
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      flushPara();
      blocks.push({ t: 'h', level: h[1]!.length as 1 | 2 | 3, c: parseInline(h[2]!) });
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    if (bullet || numbered) {
      flushPara();
      const kind = bullet ? 'ul' : 'ol';
      const items: Inline[][] = [];
      let m: RegExpExecArray | null = bullet ?? numbered;
      while (m) {
        let text = m[1]!;
        // A continuation line (indented, not itself an item) belongs to the item.
        while (n + 1 < lines.length && /^\s{2,}\S/.test(lines[n + 1]!) && !BULLET.test(lines[n + 1]!) && !NUMBERED.test(lines[n + 1]!)) {
          text += ' ' + lines[++n]!.trim();
        }
        items.push(parseInline(text));
        const next = lines[n + 1];
        const nm: RegExpExecArray | null = next === undefined ? null : kind === 'ul' ? BULLET.exec(next) : NUMBERED.exec(next);
        if (nm) n++;
        m = nm;
      }
      blocks.push(kind === 'ul' ? { t: 'ul', items } : { t: 'ol', items });
      continue;
    }
    if (/^\s{0,3}>\s?/.test(line)) {
      flushPara();
      const q: string[] = [line.replace(/^\s{0,3}>\s?/, '')];
      while (n + 1 < lines.length && /^\s{0,3}>\s?/.test(lines[n + 1]!)) q.push(lines[++n]!.replace(/^\s{0,3}>\s?/, ''));
      blocks.push({ t: 'quote', c: parseInline(q.join(' ')) });
      continue;
    }
    para.push(line.trim());
  }
  flushPara();
  return blocks;
}

/** The text of a markdown source without formatting, for a one-line preview. */
export function plainText(src: string): string {
  const text = (inl: Inline[]): string => inl.map((x) => (x.t === 'text' || x.t === 'code' ? x.s : text(x.c))).join('');
  return parseMarkdown(src)
    .map((b) => (b.t === 'ul' || b.t === 'ol' ? b.items.map(text).join(' · ') : b.t === 'code' ? b.s : text(b.c)))
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Length of a news preview, in characters. */
export const PREVIEW_CHARS = 260;

/** The first lines of a post as plain text (cut at a word near `max`) and whether there is more. */
export function newsPreview(src: string, max = PREVIEW_CHARS): { text: string; more: boolean } {
  const all = plainText(src);
  if (all.length <= max) return { text: all, more: false };
  const cut = all.lastIndexOf(' ', max);
  return { text: `${all.slice(0, cut > max * 0.6 ? cut : max).trimEnd()}…`, more: true };
}
