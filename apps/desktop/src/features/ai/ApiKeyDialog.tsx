import { CircleAlert } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { ModelProviderRecord, ProviderPreset } from "@/api/ai";
import {
  useAddModelProvider,
  useModelProviderPresets,
  useUpdateModelProviderKey,
} from "@/api/ai-queries";
import { type ApiError, toApiError } from "@/api/errors";
import { SecretInput } from "@/components/common/SecretInput";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldDescription, FieldError, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Spinner } from "@/components/ui/spinner";
import { describedBy } from "@/features/sources/add/AddErrorMessage";
import { useReturnFocus } from "@/lib/focus";
import { DataPolicyLine } from "./DataPolicyLine";
import { useAiErrorText } from "./useAiErrorText";

export type ApiKeyDialogMode = { kind: "add" } | { kind: "replace"; provider: ModelProviderRecord };

/**
 * "Add an API key" (preset → address → key) or "Replace key". The key lives only in the form's
 * state: it goes to the facade once, as a mutation variable dropped when the call settles, and
 * is cleared on success. Closing the dialog unmounts the form, which discards it.
 */
export function ApiKeyDialog({
  mode,
  onClose,
  onAdded,
}: {
  /** null keeps the dialog closed. */
  mode: ApiKeyDialogMode | null;
  onClose: () => void;
  /** After adding: the parent shows the new backend's disclosure sheet. */
  onAdded: (record: ModelProviderRecord) => void;
}) {
  const returnFocus = useReturnFocus();
  return (
    <Dialog
      open={mode !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md" {...returnFocus}>
        {mode?.kind === "add" ? <AddKeyForm onDone={onClose} onAdded={onAdded} /> : null}
        {mode?.kind === "replace" ? (
          <ReplaceKeyForm provider={mode.provider} onDone={onClose} />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

/** API-key presets: the ones that need a key (local servers are added from their own list). */
function keyPresets(presets: ProviderPreset[]): ProviderPreset[] {
  return presets.filter((p) => p.needs_key);
}

function AddKeyForm({
  onDone,
  onAdded,
}: {
  onDone: () => void;
  onAdded: (record: ModelProviderRecord) => void;
}) {
  const { t } = useTranslation("ai");
  const presets = useModelProviderPresets();
  const add = useAddModelProvider();
  const choices = keyPresets(presets.data ?? []);
  const [presetId, setPresetId] = useState<string | null>(null);
  const preset = choices.find((p) => p.id === presetId) ?? choices[0] ?? null;
  const [baseUrl, setBaseUrl] = useState("");
  const form = useKeyForm();
  const id = useId();

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!preset) return;
    const key = form.begin();
    if (key === null) return;
    try {
      const record = await add.mutateAsync({
        preset: preset.id,
        baseUrl: preset.base_url_editable ? baseUrl.trim() || null : null,
        apiKey: key,
      });
      form.succeeded();
      toast.success(t("addKey.added", { name: record.label }));
      onDone();
      onAdded(record);
    } catch (err) {
      form.failed(err);
    } finally {
      // Drop the mutation (and the key in its variables) from the mutation cache now.
      add.reset();
    }
  }

  return (
    <form onSubmit={submit} noValidate aria-busy={form.pending} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>{t("addKey.title")}</DialogTitle>
        <DialogDescription>{t("addKey.description")}</DialogDescription>
      </DialogHeader>

      <Field>
        <FieldLabel htmlFor={`${id}-preset`}>{t("addKey.provider")}</FieldLabel>
        <Select value={preset?.id ?? ""} onValueChange={setPresetId}>
          <SelectTrigger id={`${id}-preset`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {choices.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {preset ? <DataPolicyLine facts={preset.data_policy} name={preset.label} /> : null}
      </Field>

      {preset?.base_url_editable ? (
        <Field>
          <FieldLabel htmlFor={`${id}-url`}>{t("addKey.baseUrl")}</FieldLabel>
          <Input
            id={`${id}-url`}
            type="url"
            inputMode="url"
            autoComplete="off"
            spellCheck={false}
            value={baseUrl}
            placeholder={preset.default_base_url ?? "https://"}
            onChange={(e) => setBaseUrl(e.target.value)}
            aria-describedby={`${id}-url-hint`}
          />
          <FieldDescription id={`${id}-url-hint`}>{t("addKey.baseUrlHint")}</FieldDescription>
        </Field>
      ) : null}

      <KeyField form={form} />
      <KeyFormError form={form} />
      <Footer pending={form.pending} submit={t("addKey.submit")} />
    </form>
  );
}

function ReplaceKeyForm({
  provider,
  onDone,
}: {
  provider: ModelProviderRecord;
  onDone: () => void;
}) {
  const { t } = useTranslation("ai");
  const update = useUpdateModelProviderKey();
  const form = useKeyForm();

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const key = form.begin();
    if (key === null) return;
    try {
      await update.mutateAsync({ providerId: provider.provider_id, apiKey: key });
      form.succeeded();
      toast.success(t("replaceKey.done"));
      onDone();
    } catch (err) {
      form.failed(err);
    } finally {
      update.reset();
    }
  }

  return (
    <form onSubmit={submit} noValidate aria-busy={form.pending} className="grid gap-4">
      <DialogHeader>
        <DialogTitle>{t("replaceKey.title", { name: provider.label })}</DialogTitle>
        <DialogDescription>{t("addKey.description")}</DialogDescription>
      </DialogHeader>
      <KeyField form={form} />
      <KeyFormError form={form} />
      <Footer pending={form.pending} submit={t("replaceKey.submit")} />
    </form>
  );
}

// ----- the key field, shared by both forms ---------------------------------------------------

type KeyForm = ReturnType<typeof useKeyForm>;

function useKeyForm() {
  const id = useId();
  // The key: only in this state, cleared on success.
  const [key, setKey] = useState("");
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);
  const [pending, setPending] = useState(false);
  const ids = {
    input: `${id}-key`,
    hint: `${id}-hint`,
    missing: `${id}-missing`,
    error: `${id}-error`,
  };
  return {
    key,
    setKey,
    missing,
    error,
    pending,
    ids,
    /** The trimmed key to send, or null (nothing typed, or a call already running). */
    begin(): string | null {
      if (pending) return null;
      const value = key.trim();
      setMissing(value === "");
      if (value === "") return null;
      setError(null);
      setPending(true);
      return value;
    },
    succeeded() {
      setKey("");
      setPending(false);
    },
    failed(err: unknown) {
      setError(toApiError(err));
      setPending(false);
      document.getElementById(ids.input)?.focus();
    },
  };
}

function KeyField({ form }: { form: KeyForm }) {
  const { t } = useTranslation("ai");
  const { t: tc } = useTranslation();
  const invalid = form.missing || form.error ? true : undefined;
  return (
    <Field data-invalid={invalid}>
      <FieldLabel htmlFor={form.ids.input}>{t("addKey.key")}</FieldLabel>
      <SecretInput
        id={form.ids.input}
        value={form.key}
        onChange={(e) => form.setKey(e.target.value)}
        aria-invalid={invalid}
        aria-describedby={describedBy(
          form.ids.hint,
          form.missing && form.ids.missing,
          form.error && form.ids.error,
        )}
      />
      <FieldDescription id={form.ids.hint}>{t("addKey.keyHint")}</FieldDescription>
      {form.missing ? <FieldError id={form.ids.missing}>{tc("errors.invalid")}</FieldError> : null}
    </Field>
  );
}

function KeyFormError({ form }: { form: KeyForm }) {
  const { t } = useTranslation("ai");
  const errorText = useAiErrorText();
  const error = form.error;
  if (!error) return null;
  // A coding-plan key: our explanation, then the vendor's own sentence as a quote (data).
  if (error.blocked === "coding_plan_key") {
    return (
      <Alert variant="destructive" id={form.ids.error}>
        <CircleAlert aria-hidden />
        <AlertTitle>{t("addKey.codingPlan")}</AlertTitle>
        {error.message ? (
          <AlertDescription>
            <p>{t("addKey.vendorSays")}</p>
            <blockquote lang="en" className="border-l-2 border-rule pl-3 italic">
              {error.message}
            </blockquote>
          </AlertDescription>
        ) : null}
      </Alert>
    );
  }
  return (
    <Alert variant="destructive" id={form.ids.error}>
      <CircleAlert aria-hidden />
      <AlertTitle>{errorText(error)}</AlertTitle>
      {/* The backend's message (English, never a secret) as the detail. */}
      {error.message ? <AlertDescription lang="en">{error.message}</AlertDescription> : null}
    </Alert>
  );
}

function Footer({ pending, submit }: { pending: boolean; submit: string }) {
  const { t } = useTranslation("ai");
  const { t: tc } = useTranslation();
  return (
    <div className="flex justify-end gap-2">
      <DialogClose asChild>
        <Button type="button" variant="outline">
          {tc("actions.cancel")}
        </Button>
      </DialogClose>
      <Button
        type="submit"
        aria-disabled={pending || undefined}
        className="aria-disabled:opacity-50"
      >
        {pending ? <Spinner aria-hidden /> : null}
        {pending ? t("addKey.checking") : submit}
      </Button>
    </div>
  );
}
