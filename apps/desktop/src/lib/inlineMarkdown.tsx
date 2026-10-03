import type { ReactNode } from "react";

/**
 * The inline markdown a model writes in a paragraph (`**bold**`, `*italic*`, `` `code` ``) as
 * React elements. Everything else stays text: no HTML, links or images from model output.
 */
export function inlineMarkdown(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /(\*\*[^*]+\*\*|\*[^*\s][^*]*\*|`[^`]+`)/g;
  let last = 0;
  let key = 0;
  for (const match of text.matchAll(pattern)) {
    const token = match[0];
    const at = match.index;
    if (at > last) out.push(text.slice(last, at));
    if (token.startsWith("**")) out.push(<strong key={key++}>{token.slice(2, -2)}</strong>);
    else if (token.startsWith("`")) out.push(<code key={key++}>{token.slice(1, -1)}</code>);
    else out.push(<em key={key++}>{token.slice(1, -1)}</em>);
    last = at + token.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
