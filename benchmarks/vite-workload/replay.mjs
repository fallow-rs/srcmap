// The replay of recorded Vite source map work. Each function takes the `remapping`
// implementation to run, so that one recorded workload can time several of them.

/** The combine step of Vite: one `remapping` call for a list of maps, outermost first. */
const combine = (remapping, list) => {
  if (list.every((map) => map.sources.length === 0))
    return { names: [], sources: [], mappings: "", version: 3 };
  const copies = list.map((map) => ({ ...map, sources: [...map.sources] }));
  const singleSource = copies.slice(0, -1).every((map) => map.sources.length === 1);
  if (singleSource) return remapping(copies, () => null);
  let next = 1;
  return remapping(copies[0], () => copies[next++] ?? null);
};

/**
 * Walks a plugin chain the way the Vite plugin container does. Returns the maps to
 * combine, innermost first, or null when a map without `version` ends the chain with no combine.
 */
export const chainMaps = (record) => {
  const maps = record.combined ? [record.combined] : [];
  for (let map of record.maps) {
    if (typeof map === "string") map = JSON.parse(map);
    if (!("version" in map)) return null;
    if (maps.length === 0 && map.sources.length === 1 && !map.sources[0]) {
      map = { ...map, sources: [record.file] };
    }
    maps.push(map);
  }
  return maps;
};

/** Vite today: one combine per plugin map, each on the result of the last combine. */
export const replayPairwise = (remapping) => (record) => {
  if (record.kind === "pair") return combine(remapping, record.maps);
  const maps = chainMaps(record);
  if (!maps || maps.length < 2) return null;
  let combined = maps[0];
  for (let i = 1; i < maps.length; i++) combined = combine(remapping, [maps[i], combined]);
  return combined;
};

/** One combine for the full chain. A chain with a multi-source link falls back to pairwise. */
export const replayOneCall = (remapping) => {
  const pairwise = replayPairwise(remapping);
  return (record) => {
    if (record.kind === "pair") return combine(remapping, record.maps);
    const maps = chainMaps(record);
    if (!maps || maps.length < 2) return null;
    const outermostFirst = maps.toReversed();
    if (!outermostFirst.slice(0, -1).every((map) => map.sources.length === 1))
      return pairwise(record);
    return combine(remapping, outermostFirst);
  };
};
