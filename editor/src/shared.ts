// What the workspace's views share: the model path a person last picked on
// a card or in a selection (the card shows it), and the path they last
// chose to change (the change view fills it in). A view sets one, the
// others follow.

import { useSyncExternalStore } from "react";

export interface Shared {
  /** The path the resolution card is about, such as `entities.slug[1]`. */
  cardPath?: string;
  /** The model path the change view is editing, such as `entities.player.jump.gravity`. */
  changePath?: string;
}

let state: Shared = {};
const listeners = new Set<() => void>();

export function getShared(): Shared {
  return state;
}

export function setShared(patch: Partial<Shared>): void {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}

function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useShared(): Shared {
  return useSyncExternalStore(subscribe, getShared, getShared);
}
