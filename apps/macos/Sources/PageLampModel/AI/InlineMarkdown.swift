// The Markdown subset an explanation's paragraphs and check questions use (the Tauri app's
// inlineMarkdown): **strong**, *emphasis* and `code`; everything else stays literal text, so
// links, images and HTML never become live (the facade promises "a subset, never raw HTML").
// Not AttributedString(markdown:): it would make links clickable and read _x_, ~~x~~ and
// escapes the Tauri app shows as written.

import Foundation

public enum InlineMarkdown {
    /// One run of text and how it's set.
    public struct Run: Equatable, Sendable {
        public enum Style: Equatable, Sendable {
            case plain
            case strong
            case emphasis
            case code
        }

        public let text: String
        public let style: Style

        public init(_ text: String, _ style: Style) {
            self.text = text
            self.style = style
        }
    }

    /// `text` as runs, the markers removed from the styled ones.
    public static func runs(_ text: String) -> [Run] {
        var runs: [Run] = []
        var rest = text[...]
        while let match = rest.firstMatch(of: token) {
            if match.range.lowerBound > rest.startIndex {
                runs.append(Run(String(rest[rest.startIndex..<match.range.lowerBound]), .plain))
            }
            let marked = String(rest[match.range])
            if marked.hasPrefix("**") {
                runs.append(Run(String(marked.dropFirst(2).dropLast(2)), .strong))
            } else if marked.hasPrefix("`") {
                runs.append(Run(String(marked.dropFirst().dropLast()), .code))
            } else {
                runs.append(Run(String(marked.dropFirst().dropLast()), .emphasis))
            }
            rest = rest[match.range.upperBound...]
        }
        if !rest.isEmpty {
            runs.append(Run(String(rest), .plain))
        }
        return runs
    }

    /// The Tauri app's pattern: `**…**`, `*…*` (not starting with a space), `` `…` ``.
    private static var token: Regex<Substring> {
        /\*\*[^*]+\*\*|\*[^*\s][^*]*\*|`[^`]+`/
    }
}
