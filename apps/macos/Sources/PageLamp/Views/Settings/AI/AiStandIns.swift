// Offscreen renders (the snapshot harness, `\.drawsControlStandIns`) can't draw text fields,
// progress bars or link buttons: these stand in for them there, drawn from shapes and text.

import SwiftUI

/// A text field's box with its text, or its prompt dimmed (a secure field: empty).
struct FieldStandIn: View {
    let text: String
    var prompt = ""

    var body: some View {
        Text(verbatim: text.isEmpty ? prompt : text)
            .foregroundStyle(text.isEmpty ? AnyShapeStyle(.tertiary) : AnyShapeStyle(.primary))
            .lineLimit(1)
            .frame(maxWidth: .infinity, minHeight: 16, alignment: .leading)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(.background, in: .rect(cornerRadius: 5))
            .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(.separator))
    }
}

/// A linear progress bar filled to `fraction` (0…1).
struct BarStandIn: View {
    let fraction: Double

    var body: some View {
        Capsule()
            .fill(.fill.tertiary)
            .overlay(alignment: .leading) {
                GeometryReader { geometry in
                    Capsule()
                        .fill(.tint)
                        .frame(width: geometry.size.width * min(max(fraction, 0), 1))
                }
            }
            .frame(height: 6)
    }
}
