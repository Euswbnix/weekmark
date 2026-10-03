// The draft to review (design §5.1, §7; the Tauri app's PlanDraft): by day and course, with what
// PageLamp left out and why, which courses were planned from structure only, and the AI-generated
// line. The sheet's buttons (with Write Again's own "≈ $x") use, rewrite or discard it.

import SwiftUI
import PageLampKit
import PageLampModel

struct PlanDraftView: View {
    let draft: GeneratedStudyPlan
    let text: PlanText
    /// The draft replaces the run (and its Stop): VoiceOver goes to its heading.
    let titleFocus: AccessibilityFocusState<PlanSheet.Focus?>.Binding

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            VStack(alignment: .leading, spacing: PLSpace.s1) {
                Text(l10n("plan.draft.title"))
                    .font(PLType.headline.font)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityFocused(titleFocus, equals: .draft)
                Text(text.summary(draft.plan))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                Text(l10n.aiLabel(draft.meta, calendar: model.calendar))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if !draft.warnings.isEmpty || structureOnly != nil {
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    ForEach(Array(draft.warnings.enumerated()), id: \.offset) { _, warning in
                        Label {
                            Text(text.warning(warning))
                                .fixedSize(horizontal: false, vertical: true)
                        } icon: {
                            Image(systemName: "exclamationmark.triangle")
                                .foregroundStyle(PLColor.warning)
                                .accessibilityHidden(true)
                        }
                    }
                    if let structureOnly {
                        Text(structureOnly)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .font(PLType.callout.font)
            }

            let days = PlanText.days(draft.plan.items)
            if days.isEmpty {
                Text(l10n("plan.draft.noItems"))
                    .foregroundStyle(.secondary)
            } else {
                table(days)
            }

            if !draft.unscheduled.isEmpty {
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    Text(l10n("plan.unscheduled.title"))
                        .font(PLType.body.font.weight(.semibold))
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(draft.unscheduled.enumerated()), id: \.offset) { _, task in
                        unscheduledRow(task)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - The table

    /// Day · course · task · time; each day's name once, on its first task.
    private func table(_ days: [(date: String, items: [StudyPlanItem])]) -> some View {
        Grid(alignment: .leading, horizontalSpacing: PLSpace.s3, verticalSpacing: PLSpace.s2) {
            GridRow {
                header("plan.draft.day")
                header("plan.draft.course")
                header("plan.draft.task")
                header("plan.draft.time").gridColumnAlignment(.trailing)
            }
            Divider()
            ForEach(days, id: \.date) { day in
                ForEach(Array(day.items.enumerated()), id: \.offset) { index, item in
                    GridRow(alignment: .firstTextBaseline) {
                        Text(index == 0 ? text.day(day.date) : "")
                            .font(PLType.body.font.weight(.semibold))
                            .fixedSize()
                            .accessibilityHidden(index != 0)
                        // A course's code, or its name when it has none (wrapped, never wider).
                        Text(courseLabel(item.courseId) ?? "")
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: Self.courseWidth, alignment: .leading)
                        // The task takes the width left; the other columns keep theirs.
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.title)
                                .fixedSize(horizontal: false, vertical: true)
                            if let description = item.description, !description.isEmpty {
                                Text(description)
                                    .font(PLType.callout.font)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        Text(item.minutes.map(text.minutes) ?? "")
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                            .fixedSize()
                    }
                }
                Divider()
            }
        }
        // Modifiers go on the grid, not on a GridRow (VoiceOver reads the column names, then each
        // task cell by cell).
        .accessibilityElement(children: .contain)
        .accessibilityLabel(l10n("plan.draft.title"))
    }

    private func header(_ key: String) -> some View {
        Text(l10n(key))
            .font(PLType.callout.font)
            .foregroundStyle(.secondary)
            .accessibilityAddTraits(.isHeader)
    }

    private func unscheduledRow(_ task: UnscheduledTask) -> some View {
        var parts: [Text] = []
        if let course = courseLabel(task.courseId) {
            parts.append(Text(course).foregroundStyle(.secondary))
            parts.append(Text(verbatim: " · ").foregroundStyle(.secondary))
        }
        parts.append(Text(task.title))
        // " (reason)" / "（原因）": the space is the language's.
        parts.append(Text(l10n("mac.plan.reason", ["reason": text.reason(task.reason)])).foregroundStyle(.secondary))
        return InlineText.joined(parts)
            .font(PLType.callout.font)
            .fixedSize(horizontal: false, vertical: true)
    }

    // MARK: - Helpers

    private func courseLabel(_ id: String?) -> String? {
        ThisWeekContents.planCourseLabel(id, courses: model.courses)
    }

    /// The courses planned from their structure only (no material text was shared).
    private var structureOnly: String? {
        text.structureOnly(draft.meta.context.courses.filter { !$0.textIncluded }.compactMap { courseLabel($0.courseId) })
    }

    private static let courseWidth: CGFloat = 140
}
