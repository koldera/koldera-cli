// Live smoke of the koldera binary against a real koldera-core. Synthetic inputs only, written to a temp folder.
// KOLDERA_URL=http://localhost:8080 KOLDERA_TOKEN=... node scripts/e2e-live.mjs [path/to/koldera]
// Without KOLDERA_URL and KOLDERA_TOKEN it prints "skipped" and exits 0; the default binary is target/debug/koldera.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

if (!process.env.KOLDERA_URL || !process.env.KOLDERA_TOKEN) {
  console.log("skipped: set KOLDERA_URL and KOLDERA_TOKEN to run against a real koldera-core");
  process.exit(0);
}
const bin = process.argv[2] ?? path.join("target", "debug", process.platform === "win32" ? "koldera.exe" : "koldera");
const dir = mkdtempSync(path.join(tmpdir(), "koldera-e2e-"));

// A one-page untagged PDF with invented text and a non-embedded font, so the check has something to find.
function syntheticPdf() {
  const text = "BT /F1 18 Tf 72 740 Td (Synthetic sample report, fictional) Tj /F1 11 Tf 0 -28 Td (Example Co. is invented. No real data.) Tj ET";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    `<< /Length ${text.length} >>\nstream\n${text}\nendstream`,
    "<< /Title (Synthetic sample report) >>",
  ];
  let out = "%PDF-1.7\n";
  const offsets = objects.map((body, i) => {
    const at = out.length;
    out += `${i + 1} 0 obj\n${body}\nendobj\n`;
    return at;
  });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, "0")} 00000 n \n`).join("")}`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R /Info 6 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, "latin1");
}

const inputs = {
  "synthetic.pdf": syntheticPdf(),
  "synthetic.html": '<!doctype html><html lang="en"><head><title>Synthetic page</title></head><body><h1>Synthetic page</h1><p>Invented text.</p></body></html>',
  "synthetic.json": JSON.stringify({ title: "Synthetic note", language: "en", body: [{ type: "heading", level: 1, text: "Synthetic note" }, { type: "paragraph", content: ["Invented text."] }] }),
};
for (const [name, content] of Object.entries(inputs)) writeFileSync(path.join(dir, name), content);

const runs = [
  ["pdf analyze", ["pdf", "synthetic.pdf"]],
  ["pdf fix", ["pdf", "synthetic.pdf", "--fix", "--language", "en"], "remediated"],
  ["html", ["html", "synthetic.html", "--analyze-only"], "generated_pdf"],
  ["document", ["document", "synthetic.json"], "generated_pdf"],
];
let failed = false;
for (const [label, args, kind] of runs) {
  const out = path.join(dir, label.replace(" ", "-"));
  const run = spawnSync(path.resolve(bin), [...args.map((a) => (a.startsWith("synthetic") ? path.join(dir, a) : a)), "--out", out, "--json"], { encoding: "utf8" });
  // 0 = machine check passed, 2 = completed with failed machine rules; both are a working round trip.
  let result;
  try {
    result = JSON.parse(run.stdout);
  } catch {
    console.log(`FAIL ${label}: exit ${run.status}, no JSON: ${run.stderr.trim()}`);
    failed = true;
    continue;
  }
  const problems = [];
  if (![0, 2].includes(run.status)) problems.push(`exit ${run.status}`);
  if (result.job?.state !== "completed") problems.push(`state ${result.job?.state}`);
  if (result.job?.manualReviewRequired !== true) problems.push("manualReviewRequired is not true");
  for (const artifact of result.artifacts ?? []) {
    const sha = createHash("sha256").update(readFileSync(artifact.path)).digest("hex");
    if (sha !== artifact.sha256) problems.push(`${artifact.kind} sha256 mismatch`);
  }
  if (kind && !(result.artifacts ?? []).some((a) => a.kind === kind)) problems.push(`no ${kind} artifact`);
  failed ||= problems.length > 0;
  const shas = (result.artifacts ?? []).filter((a) => a.kind === kind).map((a) => a.sha256);
  console.log(`${problems.length ? "FAIL" : "ok  "} ${label}: job ${result.job?.id} exit ${run.status} validation ${result.job?.validation?.status} remediation ${result.job?.remediationStatus} ${shas.join(" ")} ${problems.join("; ")}`);
}
rmSync(dir, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
