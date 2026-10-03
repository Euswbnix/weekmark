import { Sparkles } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { useAiStatus } from "@/api/ai-queries";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { paths } from "@/lib/routes";
import { useFocusOnMount } from "@/lib/useFocusOnMount";
import { useUiStore } from "@/stores/ui";

/**
 * After the first sync, beside "Remind me": "Let PageLamp write plans and explanations", with
 * Set up a model (Settings → AI models) and Not now. Asked once: either answer is final, and it
 * isn't shown to a student who already has a model ready. The caller hides it while the AI
 * screens are off (AI_SETUP_ENABLED).
 */
export function AiOfferCard() {
  const { t } = useTranslation("onboarding");
  const status = useAiStatus();
  const setAnswered = useUiStore((s) => s.setAiOfferAnswered);
  // Read once: answering here keeps the card (with its note) until the student moves on.
  const [answeredBefore] = useState(() => useUiStore.getState().aiOfferAnswered);
  const [declined, setDeclined] = useState(false);
  const titleId = useId();
  if (answeredBefore || !status.data) return null;
  if (!declined && status.data.backends.some((b) => b.state === "ready")) return null;

  return (
    <section aria-labelledby={titleId}>
      <Card>
        <CardHeader>
          <h2 id={titleId} className="flex items-center gap-2 font-heading text-base font-medium">
            <Sparkles className="size-4" aria-hidden />
            {t("aiOffer.title")}
          </h2>
        </CardHeader>
        <CardContent className="space-y-3">
          {declined ? (
            <Declined />
          ) : (
            <>
              <p className="text-sm">{t("aiOffer.body")}</p>
              <div className="flex flex-wrap gap-2">
                <Button asChild>
                  <Link to={paths.aiSettings} onClick={setAnswered}>
                    {t("aiOffer.setUp")}
                  </Link>
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setAnswered();
                    setDeclined(true);
                  }}
                >
                  {t("aiOffer.notNow")}
                </Button>
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

/** Replaces the buttons, so it takes the focus. */
function Declined() {
  const { t } = useTranslation("onboarding");
  const ref = useFocusOnMount<HTMLParagraphElement>();
  return (
    <p ref={ref} tabIndex={-1} role="status" className="text-sm text-muted-foreground outline-none">
      {t("aiOffer.later")}
    </p>
  );
}
