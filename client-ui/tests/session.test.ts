import { describe, expect, test } from "nsh/test";
import { initialSession, reduceSession, type Session, type SessionEvent } from "../src/domain/session";
import type { DaemonRef } from "../src/domain/types";

const studio: DaemonRef = { name: "studio", url: "http://127.0.0.1:8080" };
const other: DaemonRef = { name: "other", url: "http://127.0.0.1:9090" };

const run = (start: Session, ...events: SessionEvent[]) => {
  let session = start;
  const commands = [];
  for (const event of events) {
    const [next, command] = reduceSession(session, event);
    session = next;
    commands.push(command);
  }
  return { session, commands };
};

describe("Session", () => {
  test("a single discovered Daemon is selected without a tap", () => {
    const { session } = run(initialSession, { type: "daemons", daemons: [studio] });
    expect(session).toEqual({ kind: "unpaired", daemon: studio, notice: null });
  });

  test("several Daemons wait for a choice, then ask for the key", () => {
    const listed = run(initialSession, { type: "daemons", daemons: [studio, other] });
    expect(listed.session).toEqual({ kind: "discovering", daemons: [studio, other] });
    const chosen = run(listed.session, { type: "select", daemon: other });
    expect(chosen.session).toEqual({ kind: "unpaired", daemon: other, notice: null });
  });

  test("submitting the key pairs once and the Token is saved for good", () => {
    const { session, commands } = run(
      { kind: "unpaired", daemon: studio, notice: null },
      { type: "submitKey", key: " sam01101 " },
      { type: "pairSucceeded", token: "tok-1" },
    );
    expect(commands[0]).toEqual({ type: "pair", daemon: studio, key: "sam01101" });
    expect(session).toEqual({ kind: "paired", daemon: studio, token: "tok-1" });
    expect(commands[1]).toEqual({ type: "saveToken", daemon: studio, token: "tok-1" });
  });

  test("an empty key never reaches the network", () => {
    const { session, commands } = run({ kind: "unpaired", daemon: studio, notice: null }, { type: "submitKey", key: "  " });
    expect(commands[0]).toEqual({ type: "none" });
    expect(session.kind).toBe("unpaired");
    expect(session.kind === "unpaired" && session.notice?.kind).toBe("rejected");
  });

  test("a second submit while pairing sends nothing", () => {
    const pairing: Session = { kind: "pairing", daemon: studio, key: "k" };
    const { session, commands } = run(pairing, { type: "submitKey", key: "k" });
    expect(session).toBe(pairing);
    expect(commands[0]).toEqual({ type: "none" });
  });

  test("a wrong key returns to unpaired with the reason, and retry pairs again", () => {
    const failed = run(
      { kind: "pairing", daemon: studio, key: "bad" },
      { type: "pairFailed", kind: "rejected", message: "Pairing Key 不正确" },
    );
    expect(failed.session).toEqual({ kind: "unpaired", daemon: studio, notice: { kind: "rejected", message: "Pairing Key 不正确" } });
    const retry = run(failed.session, { type: "submitKey", key: "good" });
    expect(retry.commands[0]).toEqual({ type: "pair", daemon: studio, key: "good" });
  });

  test("a saved Token restores straight to paired with no command", () => {
    const { session, commands } = run(initialSession, { type: "restored", daemon: studio, token: "old" });
    expect(session).toEqual({ kind: "paired", daemon: studio, token: "old" });
    expect(commands[0]).toEqual({ type: "none" });
  });

  test("401 clears the Token and asks to pair again", () => {
    const { session, commands } = run({ kind: "paired", daemon: studio, token: "old" }, { type: "unauthorized" });
    expect(session.kind).toBe("unpaired");
    expect(session.kind === "unpaired" && session.notice?.kind).toBe("revoked");
    expect(commands[0]).toEqual({ type: "clearToken", daemon: studio });
  });

  test("401 outside paired changes nothing", () => {
    const start: Session = { kind: "unpaired", daemon: studio, notice: null };
    expect(run(start, { type: "unauthorized" }).session).toBe(start);
  });

  test("forgetting the Daemon clears the Token and goes back to discovery", () => {
    const { session, commands } = run({ kind: "paired", daemon: studio, token: "t" }, { type: "forget", known: [studio, other] });
    expect(session).toEqual({ kind: "discovering", daemons: [studio, other] });
    expect(commands[0]).toEqual({ type: "clearToken", daemon: studio });
  });

  test("forgetting with one known Daemon (the web page) lands on its key form", () => {
    const { session } = run({ kind: "paired", daemon: studio, token: "t" }, { type: "forget", known: [studio] });
    expect(session).toEqual({ kind: "unpaired", daemon: studio, notice: null });
  });
});
