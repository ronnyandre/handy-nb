import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Search } from "lucide-react";
import type { CustomModelDef, HfRepoFile, HfRepoListing } from "@/bindings";
import { Dialog } from "@/components/ui/Dialog";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { MODEL_CAPABILITY_LANGUAGES } from "@/lib/constants/languages";
import { formatModelSize } from "@/lib/utils/format";
import { useModelStore } from "@/stores/modelStore";

type SourceTab = "HuggingFace" | "Url";

const LARGE_FILE_BYTES = 2 * 1024 * 1024 * 1024;

const nameFromFile = (repoId: string, path: string): string => {
  const repoName = repoId.split("/").pop() ?? repoId;
  const file = (path.split("/").pop() ?? path).replace(/\.(gguf|bin)$/i, "");
  const label = `${repoName} ${file.replace(/^ggml-model-?/i, "")}`
    .replace(/[-_]+/g, " ")
    .trim();
  return label.replace(/\b\w/g, (c) => c.toUpperCase());
};

interface CustomModelDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** When set, the dialog edits this model's name and language. */
  editing?: CustomModelDef | null;
}

export const CustomModelDialog: React.FC<CustomModelDialogProps> = ({
  open,
  onOpenChange,
  editing = null,
}) => {
  const { t } = useTranslation();
  const { listHfRepoFiles, addCustomModel, updateCustomModel } =
    useModelStore();

  const [tab, setTab] = useState<SourceTab>("HuggingFace");
  const [repoId, setRepoId] = useState("");
  const [listing, setListing] = useState<HfRepoListing | null>(null);
  const [selectedFile, setSelectedFile] = useState<HfRepoFile | null>(null);
  const [url, setUrl] = useState("");
  const [sha256, setSha256] = useState("");
  const [name, setName] = useState("");
  const [languageHint, setLanguageHint] = useState("");
  const [lookingUp, setLookingUp] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setTab("HuggingFace");
    setRepoId("");
    setListing(null);
    setSelectedFile(null);
    setUrl("");
    setSha256("");
    setName(editing?.name ?? "");
    setLanguageHint(editing?.language_hint ?? "");
    setError(null);
  }, [open, editing]);

  const languageOptions = useMemo(
    () =>
      [...MODEL_CAPABILITY_LANGUAGES].sort((a, b) =>
        a.label.localeCompare(b.label),
      ),
    [],
  );

  const lookUpRepo = async () => {
    if (!repoId.trim()) return;
    setLookingUp(true);
    setError(null);
    setListing(null);
    setSelectedFile(null);
    try {
      const result = await listHfRepoFiles(repoId.trim());
      setListing(result);
      if (result.files.length === 0) {
        setError(t("settings.models.custom.errors.noFiles"));
      } else if (result.files.length === 1) {
        pickFile(result, result.files[0]);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLookingUp(false);
    }
  };

  const pickFile = (repo: HfRepoListing, file: HfRepoFile) => {
    setSelectedFile(file);
    setName(nameFromFile(repo.repo_id, file.path));
  };

  const canSave = editing
    ? name.trim().length > 0
    : name.trim().length > 0 &&
      (tab === "HuggingFace"
        ? listing !== null && selectedFile !== null
        : url.trim().length > 0);

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    const hint = languageHint || null;
    try {
      if (editing) {
        await updateCustomModel(editing.id, name.trim(), hint);
      } else if (tab === "HuggingFace" && listing && selectedFile) {
        await addCustomModel({
          type: "HuggingFace",
          name: name.trim(),
          repo_id: listing.repo_id,
          revision: listing.revision,
          filename: selectedFile.path,
          size_bytes: selectedFile.size_bytes,
          language_hint: hint,
        });
      } else {
        await addCustomModel({
          type: "Url",
          name: name.trim(),
          url: url.trim(),
          sha256: sha256.trim() || null,
          language_hint: hint,
        });
      }
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const tabClass = (active: boolean) =>
    `flex-1 px-3 py-1.5 text-sm font-medium rounded-md transition-colors ${
      active
        ? "bg-logo-primary/20 text-logo-primary"
        : "text-text/60 hover:bg-mid-gray/10"
    }`;

  const labelClass = "block text-xs font-medium text-text/60 mb-1";

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      closeLabel={t("common.close")}
      title={
        editing
          ? t("settings.models.custom.editTitle")
          : t("settings.models.custom.addTitle")
      }
      description={
        editing ? undefined : t("settings.models.custom.addDescription")
      }
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("settings.models.custom.cancel")}
          </Button>
          <Button onClick={save} disabled={!canSave || saving}>
            {saving && (
              <Loader2 className="inline w-3.5 h-3.5 mr-1.5 animate-spin" />
            )}
            {editing
              ? t("settings.models.custom.save")
              : t("settings.models.custom.add")}
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        {!editing && (
          <div className="flex gap-1 p-1 rounded-lg bg-mid-gray/10">
            <button
              type="button"
              className={tabClass(tab === "HuggingFace")}
              onClick={() => {
                setTab("HuggingFace");
                setError(null);
              }}
            >
              {t("settings.models.custom.tabs.huggingFace")}
            </button>
            <button
              type="button"
              className={tabClass(tab === "Url")}
              onClick={() => {
                setTab("Url");
                setError(null);
              }}
            >
              {t("settings.models.custom.tabs.url")}
            </button>
          </div>
        )}

        {!editing && tab === "HuggingFace" && (
          <div className="space-y-3">
            <div>
              <label className={labelClass} htmlFor="custom-model-repo">
                {t("settings.models.custom.repoLabel")}
              </label>
              <div className="flex gap-2">
                <Input
                  id="custom-model-repo"
                  className="flex-1"
                  value={repoId}
                  placeholder="NbAiLab/nb-whisper-large"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  onChange={(e) => setRepoId(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void lookUpRepo();
                    }
                  }}
                />
                <Button
                  variant="secondary"
                  onClick={lookUpRepo}
                  disabled={!repoId.trim() || lookingUp}
                >
                  {lookingUp ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <Search className="w-4 h-4" />
                  )}
                </Button>
              </div>
            </div>

            {listing && listing.files.length > 0 && (
              <fieldset>
                <legend className={labelClass}>
                  {t("settings.models.custom.fileLabel")}
                </legend>
                <div className="max-h-48 overflow-y-auto rounded-md border border-mid-gray/40 divide-y divide-mid-gray/20">
                  {listing.files.map((file) => (
                    <label
                      key={file.path}
                      className={`flex items-center gap-2 px-3 py-2 text-sm cursor-pointer ${
                        selectedFile?.path === file.path
                          ? "bg-logo-primary/10"
                          : "hover:bg-mid-gray/10"
                      }`}
                    >
                      <input
                        type="radio"
                        name="custom-model-file"
                        checked={selectedFile?.path === file.path}
                        onChange={() => pickFile(listing, file)}
                      />
                      <span className="flex-1 truncate font-mono text-xs">
                        {file.path}
                      </span>
                      {file.size_bytes != null && (
                        <span className="text-xs text-text/50">
                          {formatModelSize(file.size_bytes / (1024 * 1024))}
                        </span>
                      )}
                    </label>
                  ))}
                </div>
                {selectedFile?.size_bytes != null &&
                  selectedFile.size_bytes > LARGE_FILE_BYTES && (
                    <p className="mt-1 text-xs text-warning">
                      {t("settings.models.custom.largeFileWarning")}
                    </p>
                  )}
              </fieldset>
            )}
          </div>
        )}

        {!editing && tab === "Url" && (
          <div className="space-y-3">
            <div>
              <label className={labelClass} htmlFor="custom-model-url">
                {t("settings.models.custom.urlLabel")}
              </label>
              <Input
                id="custom-model-url"
                className="w-full"
                value={url}
                placeholder="https://example.com/model.gguf"
                spellCheck={false}
                onChange={(e) => setUrl(e.target.value)}
              />
            </div>
            <div>
              <label className={labelClass} htmlFor="custom-model-sha">
                {t("settings.models.custom.sha256Label")}
              </label>
              <Input
                id="custom-model-sha"
                className="w-full font-mono"
                value={sha256}
                spellCheck={false}
                onChange={(e) => setSha256(e.target.value)}
              />
            </div>
          </div>
        )}

        <div>
          <label className={labelClass} htmlFor="custom-model-name">
            {t("settings.models.custom.nameLabel")}
          </label>
          <Input
            id="custom-model-name"
            className="w-full"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </div>

        <div>
          <label className={labelClass} htmlFor="custom-model-language">
            {t("settings.models.custom.languageLabel")}
          </label>
          <select
            id="custom-model-language"
            value={languageHint}
            onChange={(e) => setLanguageHint(e.target.value)}
            className="w-full px-3 py-2 text-sm bg-mid-gray/10 border border-mid-gray/80 rounded-md focus:outline-none focus:border-logo-primary"
          >
            <option value="">{t("settings.models.custom.languageNone")}</option>
            {languageOptions.map((lang) => (
              <option key={lang.value} value={lang.value}>
                {lang.label}
              </option>
            ))}
          </select>
          <p className="mt-1 text-xs text-text/50">
            {t("settings.models.custom.languageHelp")}
          </p>
        </div>

        {error && (
          <p role="alert" className="text-sm text-red-400">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
