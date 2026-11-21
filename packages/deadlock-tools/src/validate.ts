const SOURCE_EXTENSIONS = new Set([
  ".vmat",
  ".vmdl",
  ".vpcf",
  ".vsndevts",
  ".png",
  ".tga",
  ".psd",
  ".wav",
  ".mp3",
  ".dmx",
  ".xml",
  ".css",
  ".js",
  ".vxml",
  ".vcss",
  ".vjs",
]);

const EXECUTABLE_EXTENSIONS = new Set([
  ".exe",
  ".dll",
  ".bat",
  ".cmd",
  ".ps1",
  ".scr",
  ".sh",
  ".app",
  ".dylib",
  ".so",
  ".msi",
  ".vbs",
  ".com",
]);

const ARCHIVE_EXTENSIONS = new Set([".zip", ".7z", ".rar", ".tar", ".gz"]);

const TWO_GIGABYTES = 2 * 1024 * 1024 * 1024;

export type PayloadSeverity = "error" | "warning";

export type PayloadIssue = {
  severity: PayloadSeverity;
  code: string;
  entry?: string;
  message: string;
};

export type PayloadValidationOptions = {
  stagingFolder?: string;
};

export function validatePayload(
  entries: readonly string[],
  options: PayloadValidationOptions = {},
): PayloadIssue[] {
  const issues: PayloadIssue[] = [];

  if (entries.length === 0) {
    issues.push({
      severity: "error",
      code: "empty_payload",
      message: "Payload is empty. A mod must contain at least one compiled file.",
    });
  }

  const seen = new Map<string, string>();

  for (const raw of entries) {
    const entry = raw.trim();

    if (entry.length === 0) {
      issues.push({
        severity: "error",
        code: "empty_entry",
        entry: raw,
        message: "Entry path is empty.",
      });
      continue;
    }

    let rejected = false;

    if (entry.includes("\\")) {
      issues.push(error("backslash_path", entry, "Use forward slashes in VPK paths."));
      rejected = true;
    }

    if (entry.startsWith("/") || /^[A-Za-z]:/.test(entry)) {
      issues.push(error("absolute_path", entry, "VPK paths must be relative to the game root."));
      rejected = true;
    }

    const segments = entry.split("/");
    if (segments.some((segment) => segment === "" || segment === "." || segment === "..")) {
      issues.push(error("invalid_segment", entry, "Path has an empty, current or parent segment."));
      rejected = true;
    }

    if (options.stagingFolder !== undefined && segments[0] === options.stagingFolder) {
      issues.push(
        error(
          "staging_folder_prefix",
          entry,
          `Path starts with the staging folder "${options.stagingFolder}". Paths must start at a game folder.`,
        ),
      );
      rejected = true;
    }

    const name = segments[segments.length - 1] ?? entry;
    const lowered = name.toLowerCase();
    const extension = extensionOf(lowered);

    if (/^cache_.*\.soc$/.test(lowered) || lowered === "tools_thumbnail_cache.bin") {
      issues.push(error("tool_cache", entry, "Tool cache files are not part of a mod."));
      rejected = true;
    }

    if (lowered.endsWith(".vnmskel_c")) {
      issues.push(
        error("skeleton", entry, "Skeletons break hero animations and are not allowed in a mod."),
      );
      rejected = true;
    }

    if (segments[0]?.toLowerCase() === "scripts" && lowered.endsWith(".vdata_c")) {
      issues.push(
        error(
          "gameplay_data",
          entry,
          "Gameplay data is not cosmetic, goes stale after updates and is not allowed in a mod.",
        ),
      );
      rejected = true;
    }

    if (EXECUTABLE_EXTENSIONS.has(extension)) {
      issues.push(error("executable", entry, "Executables are never allowed in a mod payload."));
      rejected = true;
    }

    if (ARCHIVE_EXTENSIONS.has(extension)) {
      issues.push(error("archive", entry, "Archives belong outside the VPK, not inside it."));
      rejected = true;
    }

    if (!isCompiled(lowered) && !rejected) {
      if (SOURCE_EXTENSIONS.has(extension)) {
        issues.push(
          error(
            "source_file",
            entry,
            "Source assets are ignored by the game and must not ship in the payload.",
          ),
        );
      } else {
        issues.push({
          severity: "warning",
          code: "not_compiled",
          entry,
          message: "Entry does not end in _c. The game reads compiled files only.",
        });
      }
    }

    const lowerPath = entry.toLowerCase();
    if (
      lowerPath.startsWith("panorama/") &&
      lowerPath.endsWith(".vtex_c") &&
      !lowerPath.startsWith("panorama/images/")
    ) {
      issues.push({
        severity: "warning",
        code: "misplaced_panorama_image",
        entry,
        message:
          "Panorama loads images from panorama/images only. This texture sits elsewhere under panorama and the UI will not find it.",
      });
    }

    const key = entry.toLowerCase();
    const previous = seen.get(key);

    if (previous !== undefined) {
      issues.push(
        error("duplicate_path", entry, `Duplicate entry path, also listed as "${previous}".`),
      );
    } else {
      seen.set(key, entry);
    }
  }

  return issues;
}

export function hasErrors(issues: readonly PayloadIssue[]): boolean {
  return issues.some((issue) => issue.severity === "error");
}

export function needsChunking(
  totalSizeBytes: number,
  chunkSizeBytes: number = TWO_GIGABYTES,
): boolean {
  return totalSizeBytes > chunkSizeBytes;
}

export function formatPayloadIssues(issues: readonly PayloadIssue[]): string {
  if (issues.length === 0) {
    return "Payload is valid. Every entry is a compiled file at a safe game path.";
  }

  return issues
    .map((issue) => {
      const location = issue.entry === undefined ? "" : `${issue.entry}: `;
      return `${issue.severity.toUpperCase()} ${issue.code}: ${location}${issue.message}`;
    })
    .join("\n");
}

function error(code: string, entry: string, message: string): PayloadIssue {
  return { severity: "error", code, entry, message };
}

function isCompiled(name: string): boolean {
  return name.endsWith("_c");
}

function extensionOf(name: string): string {
  const index = name.lastIndexOf(".");
  return index === -1 ? "" : name.slice(index);
}
