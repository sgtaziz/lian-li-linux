import { ref } from "vue";
import { useMessage } from "naive-ui";
import { useConfigStore } from "@/stores/config";
import { useDaemonStore } from "@/stores/daemon";
import { useTranslate } from "@/i18n";

const saving = ref(false);

/** Shared save action for the header button, the unsaved bar, Ctrl+S and the close dialog. */
export function useSaveConfig() {
  const config = useConfigStore();
  const daemon = useDaemonStore();
  const message = useMessage();
  const t = useTranslate();

  async function save(): Promise<boolean> {
    if (saving.value || !config.dirty || !daemon.canWrite) return false;
    saving.value = true;
    try {
      const clean = await config.save();
      message.success(t("shell.header.saved"));
      return clean;
    } catch (error) {
      message.error(
        t("shell.header.saveFailed", { error: error instanceof Error ? error.message : String(error) }),
        { duration: 10000 },
      );
      return false;
    } finally {
      saving.value = false;
    }
  }

  return { saving, save };
}
