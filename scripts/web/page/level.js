// `?level=NAME` (docs/web.md): the model started at that level, by writing
// `model.start_level` into the bundle's model text before the module reads
// it. The model's canonical text is a JSON string in the bundle, so its
// quotes and newlines are escaped here. A null level returns the bundle as it is.
export function withLevel(bytes, level) {
  if (level === null) return bytes;
  const text = new TextDecoder().decode(bytes);
  const line = `start_level = \\"${level}\\"`;
  const set = text.replace(/start_level = \\"[^"\\]*\\"/, line);
  const out = set !== text ? set : text.replace("[model]\\n", `[model]\\n${line}\\n`);
  if (out === text && !text.includes(line)) throw new Error("this bundle's model has no [model] table to start a level in");
  return new TextEncoder().encode(out);
}
