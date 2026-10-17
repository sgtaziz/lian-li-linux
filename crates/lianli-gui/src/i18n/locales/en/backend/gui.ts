export default {
  appUpdate: { failed: "Application update failed: {error}", ready: "Update terminal is ready." },
  compatibility: {
    versionMismatch:
      "Changes are disabled: GUI/client {client} and daemon {daemon} must use matching versions. Update both and restart the selected daemon cleanly.",
    noChecks:
      "Changes are disabled: this daemon lacks compatibility checks. Update both applications and restart the selected daemon cleanly.",
    unreported:
      "Changes are disabled: this daemon cannot report compatibility. Update both applications and restart the selected daemon cleanly.",
  },
  ipc: {
    connectFailed: "cannot connect to daemon at {path}: {error}",
    responseFailed: "Daemon response failed. The request was not retried because it may have been applied: {error}",
    noCandidates: "no daemon socket candidates",
    guardSupplied: "The GUI backend supplies compatibility checks",
    compatibilityUnverified: "Cannot verify daemon compatibility: {error}",
    daemonChanged: "The daemon changed. Review copied settings before saving",
    unsupportedRequest: "Unsupported request: {error}",
    serviceBusy: "Wait for the service operation to finish before changing daemon settings.",
  },
  service: {
    progressDisappeared: "Service progress disappeared. Recheck actual services before retrying.",
    progressUnreadable: "Cannot read service progress: {error}",
    closeEditor: "Save and close the template editor before managing services.",
    busy: "Another service action is in progress.",
    observeFailed: "Cannot observe service progress: {error}",
    replaced: "Another operation replaced this progress record. Recheck actual services.",
    observeSwitchFailed: "Cannot observe switch progress: {error}",
    switchProgressDisappeared: "Switch progress disappeared. Recheck before retrying.",
    switchReplaced: "Another switch replaced the progress record. Recheck actual services.",
    hostSupportInstalled: "Host support installed. Choose which service mode to start.",
    submissionInterrupted:
      "GUI submission was interrupted. Recheck independent service progress before retrying.",
    editorBlocked: "Wait for the service operation to finish before opening the template editor.",
  },
  installation: {
    cacheUnavailable: "Installation cache is unavailable",
    alreadyRunning: "An installation check is already running. Try again shortly.",
  },
  diagnostics: {
    previewMainWindow: "Open diagnostic export from the main window",
    saveMainWindow: "Save diagnostic exports from the main window",
    logsMainWindow: "Choose daemon logs from the main window",
    previewRunning: "Another diagnostic preview is still running",
    selectDaemonLogs: "Select daemon logs. Session-worker logs alone are insufficient",
    previewTimedOut: "Diagnostic preview timed out. Repair journal access and retry: {error}",
    previewChanged: "The preview changed. Generate and review it again before saving",
    logsRequired:
      "Daemon logs are required. Restore journal access and generate another preview before saving",
  },
  managedImport: {
    daemonChanged: "The connected daemon changed. Review the selection before copying",
    submissionPending: "An import submission is already pending",
    serviceBusy: "Wait for the service operation before importing media",
    updateDaemon: "Update the daemon before copying custom-template assets",
    unknownMode: "The daemon mode is unknown. Reconnect before importing",
    noResult: "No import result is recorded",
    replaced: "This import is active or was replaced. Check progress again",
    unverified: "The import has no verified result",
  },
  mediaPreview: {
    relativePath: "Preview needs an absolute media path",
    unsupportedType: "This file type cannot be previewed",
    openFailed: "Cannot open media: {error}",
    inspectFailed: "Cannot inspect media: {error}",
    notFile: "Preview needs a regular file",
    tooLarge: "Media file is too large to preview",
    readFailed: "Cannot read media: {error}",
  },
  startupImage: {
    unsupportedExtension: "Choose a PNG, JPEG or BMP still image",
    openFailed: "Opening startup image: {error}",
    notFile: "Choose a regular image file",
    tooLarge: "Image exceeds 8 MiB",
    emptyOrTooLarge: "Choose a nonempty image under 8 MiB",
    notImage: "The selected file is not a PNG, JPEG or BMP image",
  },
  externalLink: {
    httpsOnly: "Only https links can be opened",
    hostSpawnMissing:
      "host-spawn is not installed in this box. Run distrobox-host-exec true once inside the box, then try again",
    startFailed: "Cannot start xdg-open: {error}",
    launcherFailed: "xdg-open failed ({status})",
    checkFailed: "Cannot check xdg-open: {error}",
    portalFailed: "Desktop portal failed ({portal}); {error}",
  },
};
