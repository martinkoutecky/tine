import { beforeEach } from "vitest";
import { delegateEvents, DelegatedEvents } from "solid-js/web";

beforeEach(() => {
  delegateEvents([...DelegatedEvents], document);
});
