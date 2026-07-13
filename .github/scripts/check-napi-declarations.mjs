import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const SELECTED_METHODS = ["source", "name"];
const ROOT_URL = new URL("../../", import.meta.url);
const GENERATED_DECLARATION_URL = new URL("target/napi-sourcemap.d.ts", ROOT_URL);
const PUBLIC_DECLARATION_URL = new URL("packages/sourcemap/index.d.ts", ROOT_URL);

const extractReturnType = (declaration, method) => {
  const signature = new RegExp(
    `^\\s*${method}\\s*\\(\\s*index\\s*:\\s*number\\s*\\)\\s*:\\s*([^;\\n]+)\\s*;?`,
    "m",
  ).exec(declaration);

  return signature?.[1].trim().replaceAll(/\s+/g, " ") ?? null;
};

/** Compare selected generated and public NAPI method return types. */
export const findDeclarationMismatches = (generatedDeclaration, publicDeclaration) => {
  const mismatches = [];

  for (const method of SELECTED_METHODS) {
    const generatedReturnType = extractReturnType(generatedDeclaration, method);
    const publicReturnType = extractReturnType(publicDeclaration, method);

    if (generatedReturnType === publicReturnType && generatedReturnType !== null) {
      continue;
    }

    mismatches.push(
      `${method}: generated returns ${generatedReturnType ?? "no declaration"}, public declaration returns ${publicReturnType ?? "no declaration"}`,
    );
  }

  return mismatches;
};

const main = async () => {
  const [generatedDeclaration, publicDeclaration] = await Promise.all([
    readFile(GENERATED_DECLARATION_URL, "utf8"),
    readFile(PUBLIC_DECLARATION_URL, "utf8"),
  ]);
  const mismatches = findDeclarationMismatches(generatedDeclaration, publicDeclaration);

  if (mismatches.length === 0) {
    return;
  }

  throw new Error(`NAPI declaration drift detected:\n${mismatches.join("\n")}`);
};

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
