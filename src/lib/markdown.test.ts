import { describe, expect, it } from 'vitest';
import { parseInline, parseMarkdown, plainText, safeHref } from './markdown';

describe('inline', () => {
  it('reads bold, italic, code and links', () => {
    expect(parseInline('a **b** _c_ `d` [e](https://x.org/y)')).toEqual([
      { t: 'text', s: 'a ' },
      { t: 'b', c: [{ t: 'text', s: 'b' }] },
      { t: 'text', s: ' ' },
      { t: 'i', c: [{ t: 'text', s: 'c' }] },
      { t: 'text', s: ' ' },
      { t: 'code', s: 'd' },
      { t: 'text', s: ' ' },
      { t: 'a', href: 'https://x.org/y', c: [{ t: 'text', s: 'e' }] },
    ]);
  });

  it('never makes a clickable link of anything but https', () => {
    for (const bad of ['javascript:alert(1)', 'http://x.org', 'data:text/html,x', 'file:///etc/passwd', '//evil.org', 'https://a b']) {
      const out = parseInline(`[click](${bad})`);
      expect(out.some((n) => n.t === 'a')).toBe(false);
      expect(plainText(`[click](${bad})`)).toContain('click');
    }
    expect(safeHref('https://example.org/a?b=c')).toBe('https://example.org/a?b=c');
    expect(safeHref('HTTPS://example.org')).not.toBeNull();
    expect(safeHref('https://x"onmouseover=...')).toBeNull();
  });

  it('keeps html as text', () => {
    const out = parseInline('<img src=x onerror=alert(1)> <script>alert(1)</script>');
    expect(out).toEqual([{ t: 'text', s: '<img src=x onerror=alert(1)> <script>alert(1)</script>' }]);
    const blocks = parseMarkdown('<b>hi</b>\n\n- <i>x</i>');
    expect(JSON.stringify(blocks)).not.toMatch(/"t":"(a|img)"/);
  });

  it('does not take snake_case or lone stars for emphasis, and supports escapes', () => {
    expect(parseInline('some_snake_case_name')).toEqual([{ t: 'text', s: 'some_snake_case_name' }]);
    expect(parseInline('2 * 3 * 4')).toEqual([{ t: 'text', s: '2 * 3 * 4' }]);
    expect(parseInline('\\*not bold\\*')).toEqual([{ t: 'text', s: '*not bold*' }]);
    expect(parseInline('**unclosed')).toEqual([{ t: 'text', s: '**unclosed' }]);
  });

  it('is bounded: deeply nested markers do not run away', () => {
    const nested = '**' + '_'.repeat(1) + 'a' + '_' + '**';
    expect(() => parseInline(nested.repeat(50))).not.toThrow();
  });
});

describe('blocks', () => {
  it('reads headings, lists, quotes, code and paragraphs', () => {
    const md = [
      '# Title',
      'line one',
      'line two',
      '',
      '- a',
      '- b **bold**',
      '  continued',
      '',
      '1. first',
      '2) second',
      '',
      '> quoted',
      '> text',
      '',
      '```',
      'let x = 1; <b>',
      '```',
    ].join('\n');
    const b = parseMarkdown(md);
    expect(b.map((x) => x.t)).toEqual(['h', 'p', 'ul', 'ol', 'quote', 'code']);
    expect(b[1]).toEqual({ t: 'p', c: [{ t: 'text', s: 'line one line two' }] });
    const ul = b[2]!;
    expect(ul.t === 'ul' && ul.items).toHaveLength(2);
    expect(plainText(md)).toContain('b bold continued');
    expect(b[5]).toEqual({ t: 'code', s: 'let x = 1; <b>' });
  });

  it('handles windows line ends, an unclosed fence and empty input', () => {
    expect(parseMarkdown('a\r\n\r\nb').map((x) => x.t)).toEqual(['p', 'p']);
    expect(parseMarkdown('```\nopen')).toEqual([{ t: 'code', s: 'open' }]);
    expect(parseMarkdown('')).toEqual([]);
    expect(parseMarkdown('####### not a heading').map((x) => x.t)).toEqual(['p']);
  });
});
