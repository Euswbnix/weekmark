import { render } from "@testing-library/react";
import { expect, it } from "vitest";
import { inlineMarkdown } from "./inlineMarkdown";

function html(text: string) {
  return render(<p>{inlineMarkdown(text)}</p>).container.innerHTML;
}

it("renders bold, italic and code, and nothing else", () => {
  expect(html("A **key** idea, *in short*, is `f(x)`.")).toBe(
    "<p>A <strong>key</strong> idea, <em>in short</em>, is <code>f(x)</code>.</p>",
  );
  // Markup from a model stays text; links and images are not rendered.
  expect(html('<img src=x onerror="alert(1)"> [a](https://x.test)')).toBe(
    '<p>&lt;img src=x onerror="alert(1)"&gt; [a](https://x.test)</p>',
  );
  expect(html("2 * 3 * 4")).toBe("<p>2 * 3 * 4</p>");
});
