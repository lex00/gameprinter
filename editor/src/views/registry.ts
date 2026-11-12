// The view registry. The shell (App.tsx) draws every registered view, in
// `order`, for the open recordings. A view is a module that calls
// `registerView` once; views/index.ts imports the built-in ones, and a new
// view adds one import there (docs/inspector.md, "Adding a view").

import type { ComponentType } from "react";
import type { Api, DiffJson, PairJson } from "../api";
import type { Pane } from "../pane";

export interface ViewProps {
  api: Api;
  frame: number;
  /** The recording being inspected. */
  a: Pane;
  /** The second recording, in diff mode. */
  b?: Pane;
  /** The two recordings' first difference, in diff mode. */
  diff?: DiffJson;
  /** A run of the recording's model, at the model frame that matches `frame`. */
  m?: Pane;
  /** How the model run lines up with the recording, and where they differ. */
  pair?: PairJson;
  setFrame: (frame: number) => void;
}

export interface ViewDef {
  id: string;
  title: string;
  /** Lower comes first. Built-in views use multiples of 10. */
  order: number;
  /** Takes the whole row rather than half of it. */
  wide?: boolean;
  /** Shown only when two recordings are open. */
  diffOnly?: boolean;
  component: ComponentType<ViewProps>;
}

const views: ViewDef[] = [];

export function registerView(def: ViewDef): void {
  if (views.some((v) => v.id === def.id)) throw new Error(`view ${def.id} is registered twice`);
  views.push(def);
  views.sort((x, y) => x.order - y.order || x.id.localeCompare(y.id));
}

export function registeredViews(): readonly ViewDef[] {
  return views;
}
