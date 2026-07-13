import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const bindingKind = (pkg) => {
  if (pkg.name.endsWith("-napi")) {
    return "napi";
  }

  if (pkg.name.endsWith("-wasm")) {
    return "wasm";
  }

  return null;
};

const cargoPublishIsDisabled = (pkg) => Array.isArray(pkg.publish) && pkg.publish.length === 0;

const releaseSteps = (workflow) => workflow.split(/\n {6}- name:/);

const hasReleasePath = (pkg, workflow, rootPath) => {
  const kind = bindingKind(pkg);
  const packagePath = relative(rootPath, dirname(pkg.manifest_path)).replaceAll("\\", "/");
  const buildCommand = kind === "napi" ? "napi build" : "wasm-pack build";

  return releaseSteps(workflow).some(
    (step) =>
      step.includes(buildCommand) &&
      (step.includes(`cd ${packagePath}`) || step.includes(`working-directory: ${packagePath}`)),
  );
};

export const findUnclassifiedBindings = ({ packages, releaseWorkflow, rootPath }) =>
  packages
    .filter((pkg) => bindingKind(pkg) !== null)
    .filter((pkg) => !cargoPublishIsDisabled(pkg))
    .filter((pkg) => !hasReleasePath(pkg, releaseWorkflow, rootPath))
    .map((pkg) => pkg.name)
    .sort();

export const loadBindingPolicyInputs = async (rootUrl) => {
  const rootPath = fileURLToPath(rootUrl);
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], {
      cwd: rootPath,
      encoding: "utf8",
    }),
  );
  const workspaceMembers = new Set(metadata.workspace_members);
  const releaseWorkflow = await readFile(new URL(".github/workflows/release.yml", rootUrl), "utf8");

  return {
    packages: metadata.packages.filter((pkg) => workspaceMembers.has(pkg.id)),
    releaseWorkflow,
    rootPath,
  };
};
