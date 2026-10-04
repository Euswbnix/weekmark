import { CircleAlert, LockKeyhole, TriangleAlert, UserRound } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { type ApiError, toApiError } from "@/api/errors";
import { useAddCanvasSource, useSyncPrefs } from "@/api/queries";
import { SecretInput } from "@/components/common/SecretInput";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { describedBy, useAddErrorText } from "./AddErrorMessage";
import { FormFooter } from "./FormFooter";
import type { AddFormProps } from "./types";

/**
 * "canvas.example.edu" → "https://canvas.example.edu". An address the student typed with a
 * scheme is sent as typed — an explicit http:// gets a visible warning instead of a silent fix.
 */
function withScheme(address: string): string {
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(address) ? address : `https://${address}`;
}

function isPlainHttp(address: string): boolean {
  return /^http:\/\//i.test(address.trim());
}

/**
 * Canvas with a personal access token. Shows the required personal-use notice
 * (common:canvasNotice) above the fields. The token is checked by the backend before saving.
 */
export function AddCanvasForm({ submitLabel, onAdded, footerStart }: AddFormProps) {
  const { t } = useTranslation("sources");
  const { t: tc } = useTranslation();
  const addCanvas = useAddCanvasSource();
  const syncPrefs = useSyncPrefs();
  const autoSync = syncPrefs.data !== undefined && syncPrefs.data.auto_sync !== "off";
  const errorText = useAddErrorText();
  const id = useId();

  const [baseUrl, setBaseUrl] = useState("");
  // Secret: lives only in this state (gone when the form unmounts), cleared once saved.
  const [token, setToken] = useState("");
  const [missing, setMissing] = useState({ url: false, token: false });
  const [error, setError] = useState<ApiError | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    const url = baseUrl.trim();
    const secret = token.trim();
    const nextMissing = { url: url === "", token: secret === "" };
    setMissing(nextMissing);
    if (nextMissing.url || nextMissing.token) return;

    setError(null);
    setPending(true);
    try {
      const record = await addCanvas.mutateAsync({ baseUrl: withScheme(url), token: secret });
      setToken("");
      setPending(false);
      onAdded([record]);
    } catch (err) {
      const apiError = toApiError(err);
      setError(apiError);
      setPending(false);
      // Rejected token → fix the token; anything else (bad address, unreachable) → the address.
      document.getElementById(apiError.kind === "auth" ? ids.token : ids.url)?.focus();
    } finally {
      // Drop the mutation (and the token in its variables) from the mutation cache now.
      addCanvas.reset();
    }
  }

  const ids = {
    url: `${id}-url`,
    urlHint: `${id}-url-hint`,
    urlMissing: `${id}-url-missing`,
    urlHttp: `${id}-url-http`,
    token: `${id}-token`,
    tokenHowTo: `${id}-token-how`,
    tokenNoButton: `${id}-token-no-button`,
    tokenPrivate: `${id}-token-private`,
    tokenMissing: `${id}-token-missing`,
    error: `${id}-error`,
  };

  return (
    <form onSubmit={submit} noValidate aria-busy={pending}>
      <FieldGroup>
        {/* A static notice, so role="note" instead of Alert's default live "alert" role. */}
        <Alert role="note">
          <UserRound aria-hidden />
          <AlertTitle>{t("canvasForm.noticeTitle")}</AlertTitle>
          <AlertDescription>
            <p className="font-medium text-foreground">{tc("canvasNotice")}</p>
            <p>{t("canvasForm.shareHint")}</p>
            {/* A new student adds Canvas here before ever seeing "What's new". */}
            {autoSync ? <p>{t("canvasForm.autoSync")}</p> : null}
          </AlertDescription>
        </Alert>

        <Field data-invalid={missing.url ? true : undefined}>
          <FieldLabel htmlFor={ids.url}>{t("canvasForm.urlLabel")}</FieldLabel>
          <Input
            id={ids.url}
            type="url"
            inputMode="url"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={t("canvasForm.urlPlaceholder")}
            spellCheck={false}
            autoCapitalize="off"
            aria-invalid={missing.url ? true : undefined}
            aria-describedby={describedBy(
              ids.urlHint,
              isPlainHttp(baseUrl) && ids.urlHttp,
              missing.url && ids.urlMissing,
              error && ids.error,
            )}
          />
          <FieldDescription id={ids.urlHint}>{t("canvasForm.urlHint")}</FieldDescription>
          {isPlainHttp(baseUrl) ? (
            <FieldDescription id={ids.urlHttp} className="flex items-start gap-1.5 text-foreground">
              <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warning" aria-hidden />
              {t("canvasForm.httpWarning")}
            </FieldDescription>
          ) : null}
          {missing.url ? (
            <FieldError id={ids.urlMissing}>{t("canvasForm.missingUrl")}</FieldError>
          ) : null}
        </Field>

        <Field data-invalid={missing.token ? true : undefined}>
          <FieldLabel htmlFor={ids.token}>{t("canvasForm.tokenLabel")}</FieldLabel>
          <SecretInput
            id={ids.token}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            aria-invalid={missing.token ? true : undefined}
            aria-describedby={describedBy(
              ids.tokenHowTo,
              ids.tokenNoButton,
              ids.tokenPrivate,
              missing.token && ids.tokenMissing,
              error && ids.error,
            )}
          />
          <FieldDescription id={ids.tokenHowTo}>{t("canvasForm.tokenHowTo")}</FieldDescription>
          <FieldDescription id={ids.tokenNoButton}>
            {t("canvasForm.noTokenButton")}
          </FieldDescription>
          <FieldDescription id={ids.tokenPrivate} className="flex items-start gap-1.5">
            <LockKeyhole className="mt-0.5 size-3.5 shrink-0" aria-hidden />
            {t("canvasForm.tokenPrivate")}
          </FieldDescription>
          {missing.token ? (
            <FieldError id={ids.tokenMissing}>{t("canvasForm.missingToken")}</FieldError>
          ) : null}
        </Field>

        {/* Canvas answers for address and token together, so the error belongs to the form. */}
        {error ? (
          <Alert variant="destructive" id={ids.error}>
            <CircleAlert aria-hidden />
            <AlertTitle>{errorText("canvas", error)}</AlertTitle>
            {error.message ? <AlertDescription>{error.message}</AlertDescription> : null}
          </Alert>
        ) : null}

        <FormFooter
          start={footerStart}
          pending={pending}
          submitLabel={submitLabel}
          pendingLabel={t("canvasForm.submitting")}
        />
      </FieldGroup>
    </form>
  );
}
