import { FileText, TriangleAlert } from "lucide-react";
import { type ReactNode, useId } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { EstimateRequest } from "@/api/ai";
import { useApi } from "@/api/context";
import type { Citation, WeeklyExplanation } from "@/api/explain";
import { AiGeneratedLabel, aiGeneratedLabelText } from "@/components/common/AiGeneratedLabel";
import { CopyButton } from "@/components/common/CopyButton";
import { useOpenExternal } from "@/components/common/useOpenExternal";
import { Button } from "@/components/ui/button";
import { GenerateButton } from "@/features/ai/GenerateButton";
import { inlineMarkdown } from "@/lib/inlineMarkdown";
import { isHttpUrl } from "@/lib/url";

/**
 * One explanation (design §5.2, §7): its sections, each paragraph with its citation chips (the
 * local file, else the LMS page), the check questions, what wasn't read and why, and the
 * AI-generated line, which Copy carries too. Include writes again with the left-out materials the
 * facade brings back, from "≈ $x" for exactly what it sends.
 */
export function ExplanationView({
  explanation,
  include,
  actions,
}: {
  explanation: WeeklyExplanation;
  /** More controls beside Copy (Delete). */
  actions?: ReactNode;
  /**
   * Write again with the left-out materials included: the request its "≈ $x" prices (what the
   * run sends), and a key that changes with each run (its over-budget tick is per run).
   */
  include?: {
    request: EstimateRequest;
    resetKey: string;
    onInclude: (options: { overrideBudget: boolean }) => void;
  };
}) {
  const { t, i18n } = useTranslation("explain");
  const { t: tai } = useTranslation("ai");
  // The facade says which left-out materials "include" brings back.
  const includable = explanation.left_out.filter((m) => m.includable);
  const includeNoteId = useId();

  return (
    <article className="space-y-5">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <AiGeneratedLabel meta={explanation.meta} />
        <div className="flex items-center gap-1">
          <CopyButton
            text={copyText(explanation, aiGeneratedLabelText(explanation.meta, tai, i18n.language))}
            label={t("result.copy")}
          />
          {actions}
        </div>
      </div>

      {explanation.sections.map((section, s) => (
        // Sections and paragraphs have no ids; an explanation never changes.
        // biome-ignore lint/suspicious/noArrayIndexKey: fixed list
        <section key={s} className="space-y-2">
          <h4 className="font-medium">{section.heading}</h4>
          {section.paragraphs.map((paragraph, p) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed list
            <div key={p} className="space-y-1.5">
              <p className="pl-prose text-sm">{inlineMarkdown(paragraph.text)}</p>
              <ul aria-label={t("result.sourcesLabel")} className="flex flex-wrap gap-1.5">
                {paragraph.citations.map((citation) => (
                  <li key={`${citation.handle}-${citation.locator ?? ""}`}>
                    <CitationChip citation={citation} />
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </section>
      ))}

      {explanation.dropped_citations > 0 ? (
        <p className="flex items-start gap-2 text-sm text-muted-foreground">
          <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden />
          {t("result.droppedCitations", { count: explanation.dropped_citations })}
        </p>
      ) : null}

      {explanation.check_questions.length > 0 ? (
        <section className="space-y-2">
          <h4 className="font-medium">{t("result.questions")}</h4>
          <ol className="list-decimal space-y-1 pl-5 text-sm">
            {explanation.check_questions.map((question) => (
              <li key={question}>{inlineMarkdown(question)}</li>
            ))}
          </ol>
        </section>
      ) : null}

      {explanation.left_out.length > 0 ? (
        <section className="space-y-2">
          <h4 className="text-sm font-medium">{t("result.leftOut")}</h4>
          <ul className="space-y-1 text-sm">
            {explanation.left_out.map((material) => (
              <li key={material.material_id}>
                {material.title}
                <span className="text-muted-foreground">
                  {" "}
                  ({t(`result.leftOutReason.${material.reason}`)})
                </span>
              </li>
            ))}
          </ul>
          {include && includable.length > 0 ? (
            // Correcting a title that only looks like graded work; not a way to get answers.
            <div className="space-y-1.5">
              <GenerateButton
                key={include.resetKey}
                request={include.request}
                variant="outline"
                describedBy={includeNoteId}
                // Not a plural key: zh-CN has one form, and "it" needs no number.
                label={
                  includable.length === 1
                    ? t("result.includeOne")
                    : t("result.includeSeveral", { count: includable.length })
                }
                onGenerate={include.onInclude}
              />
              <p id={includeNoteId} className="text-xs text-muted-foreground">
                {t("result.includeNote")}
              </p>
            </div>
          ) : null}
        </section>
      ) : null}

      {explanation.cite_ai_use ? (
        <p className="text-sm text-muted-foreground">{t("result.citeAiUse")}</p>
      ) : null}
    </article>
  );
}

/** "Title, locator": opens the material's local file, else its LMS page. */
function CitationChip({ citation }: { citation: Citation }) {
  const { t } = useTranslation("explain");
  const api = useApi();
  const openExternal = useOpenExternal();
  const label = citation.locator
    ? t("result.citation", { title: citation.title, locator: citation.locator })
    : citation.title;

  async function open() {
    const opened = await api.openMaterial(citation.material_id).catch(() => false);
    if (opened) return;
    if (citation.url && isHttpUrl(citation.url)) openExternal(citation.url);
    else toast.info(t("result.notOnComputer", { title: citation.title }));
  }

  return (
    <Button
      type="button"
      size="sm"
      variant="outline"
      className="h-auto rounded-full px-2.5 py-0.5 text-xs font-normal"
      onClick={() => void open()}
    >
      <FileText aria-hidden />
      {label}
    </Button>
  );
}

/** Plain text for the clipboard: the sections with their sources, the questions, the label. */
function copyText(explanation: WeeklyExplanation, label: string): string {
  const parts: string[] = [];
  for (const section of explanation.sections) {
    parts.push(section.heading);
    for (const paragraph of section.paragraphs) {
      const sources = paragraph.citations
        .map((c) => (c.locator ? `${c.title}, ${c.locator}` : c.title))
        .join("; ");
      parts.push(sources ? `${paragraph.text} [${sources}]` : paragraph.text);
    }
  }
  if (explanation.check_questions.length > 0) {
    parts.push(explanation.check_questions.map((q, i) => `${i + 1}. ${q}`).join("\n"));
  }
  parts.push(label);
  return parts.join("\n\n");
}
