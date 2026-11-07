import { describe, expect, it } from "vitest";
import { sshAuthority } from "./authority";

describe("SSH authority display", () => {
  it("keeps the host unambiguous when a user name contains @", () => {
    expect(sshAuthority("alice@corp.example", "good.example", 22)).toBe("alice%40corp.example@good.example:22");
    expect(sshAuthority("100%", "host", 2222)).toBe("100%25@host:2222");
    expect(sshAuthority("deploy", "build.example.test", 22)).toBe("deploy@build.example.test:22");
    expect(sshAuthority(null, "build.example.test", 22)).toBe("build.example.test:22");
  });

  it("brackets IPv6 literals exactly once", () => {
    expect(sshAuthority("root", "2001:db8::1", 22)).toBe("root@[2001:db8::1]:22");
    expect(sshAuthority("root", "[::1]", 22)).toBe("root@[::1]:22");
  });
});
