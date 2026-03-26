import { access } from "node:fs/promises";
import { constants } from "node:fs";
import { BinaryNotFoundError } from "./errors.js";

export interface InstallationDiagnostics {
  ok: boolean;
  binaryPath: string;
  platform: NodeJS.Platform;
  arch: string;
  checks: {
    binaryOnPath: boolean;
    explicitBinaryExists: boolean | null;
  };
  message: string;
  nextSteps: string[];
}

export async function diagnoseInstallation(binaryPath = "sandcastle"): Promise<InstallationDiagnostics> {
  const explicitBinaryExists = await canAccess(binaryPath);
  const binaryOnPath = binaryPath === "sandcastle" ? explicitBinaryExists : await canAccess("sandcastle");
  const ok = explicitBinaryExists;

  if (ok) {
    return {
      ok: true,
      binaryPath,
      platform: process.platform,
      arch: process.arch,
      checks: {
        binaryOnPath,
        explicitBinaryExists,
      },
      message: `SandCastle binary is available at "${binaryPath}".`,
      nextSteps: [],
    };
  }

  const error = new BinaryNotFoundError(binaryPath);
  return {
    ok: false,
    binaryPath,
    platform: process.platform,
    arch: process.arch,
    checks: {
      binaryOnPath,
      explicitBinaryExists,
    },
    message: error.message,
    nextSteps: error.nextSteps,
  };
}

async function canAccess(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
