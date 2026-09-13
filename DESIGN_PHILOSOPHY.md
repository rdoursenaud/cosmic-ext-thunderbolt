# Design Philosophy: Cosmic Ext Thunderbolt

## 1. Core Mission

**Cosmic Ext Thunderbolt** is not merely a graphical frontend for `boltd`. It
is an **Intelligent I/O Topology Manager** designed to bridge the gap between
the raw complexity of the Linux kernel Thunderbolt subsystem and the user's
need for security, performance, and understanding.

Our goal is to make the invisible visible: exposing physical bridges, bandwidth
bottlenecks, security layers, and driver bindings that standard tools hide,
while ensuring that powerful operations are accessible only through safe,
friction-mediated interfaces.

## 2. Guiding Principles

### 2.1. Transparency Over Abstraction

Standard tools often present a "flat" logical view (Device Connected = Good).
We reject this simplification when it hides critical context.

- **Physical Reality:** We expose the full chain: Host → Bridge → Adapter →
  Device. If a Thunderbolt 2 adapter limits a Thunderbolt 3 chain, the user
  _must_ see it.
- **Driver Visibility:** We correlate logical UUIDs with physical PCI paths
  and loaded kernel modules. If a device is connected but lacks a driver, we
  show it.
- **Security Truth:** We distinguish between "Domain Security Level" (policy)
  and "Effective Security" (capability). A domain in `Secure` mode connected
  to a legacy device is _not_ secure; we report this gap honestly.

### 2.2. Safety Through Friction

Thunderbolt is a potential "footgun" technology: it grants DMA access to
external hardware. A mistake can lead to data corruption, system instability,
or permanent hardware damage ("bricking").

- **Safe Defaults:** Destructive actions are never default. "Forgetting" a
  device defers de-authorization until physical disconnection to prevent data
  loss.
- **Contextual Blocking:** We actively prevent self-sabotage.
  - _Example:_ Updating firmware on an eGPU currently driving the display
    is **blocked** to prevent losing the GUI mid-flash.
  - _Example:_ Force de-authorization is hidden behind deep menus and
    requires explicit confirmation of data loss risks.
- **Friction as a Feature:** For expert operations (manual NVM flash, force
  unbind), we introduce deliberate friction (warnings, countdowns,
  checkboxes). This is not bad UX; it is a necessary circuit breaker.

### 2.3. Education via Interface

Every alert, tooltip, and recommendation is an opportunity to educate the user
about how Thunderbolt works.

- **Explain the "Why":** Don't just say "Speed Limited." Say "Speed Limited:
  Thunderbolt 2 Adapter detected in chain."
- **Demystify Security:** Explain the difference between IOMMU hardware
  isolation, Secure Connect cryptography, and simple UUID approval.
- **Actionable Insights:** Move from "Something is wrong" to "Move this cable
  to Port B to double the bandwidth."

### 2.4. Respect the Stack

We do not reinvent the wheel; we orchestrate it.

- **Kernel Truth:** Our backend reads the kernel's view (`sysfs`, `udev`) to
  ensure accuracy. We trust the kernel's state machine over heuristics.
- **Standard Tools:** We leverage `boltd` for ACL management and `fwupd` for
  firmware updates. We add the _context_ and _control layer_ they lack,
  rather than replacing their core logic.
- **No Magic:** We avoid "auto-fixing" complex low-level issues silently. If
  a manual UEFI/BIOS change is needed (e.g., enabling VT-d), we guide the user
  to do it, rather than attempting risky workarounds.

## 3. Security Model

### 3.1. The Hierarchy of Trust

We classify security states clearly for the user:

1.  **Hardware Isolated (IOMMU Active):** Highest trust. DMA is physically
    restricted. Legacy security levels are redundant.
2.  **Cryptographic (Secure Connect):** High trust. Device identity verified
    via challenge/response.
3.  **Software ACL (User Approval):** Medium trust. Relies on UUID uniqueness
    (vulnerable to cloning).
4.  **Legacy (None/DPOnly):** Low trust. Use with caution.

### 3.2. Protection Against "Leaky Abstractions"

When `boltd` reports "Connected," we verify the physical link state. If the
kernel reports CRC errors or link width degradation, we alert the user even if
the logical state is "OK." We do not let the abstraction layer hide physical
failures.

## 4. User Experience Tiers

We serve two distinct personas with a single interface, using **Progressive
Disclosure**:

### Tier 1: The General User (Applet & Default View)

- **Goal:** Connect devices safely and quickly.
- **Experience:** Clean, flat list. Focus on "Authorize" actions.
- **Safety:** Automatic deferral of destructive actions. Clear, simple
  language.
- **Visibility:** Only critical warnings (e.g., "Security Level Low") are
  shown.

### Tier 2: The Power User / Developer (Settings & Expert Mode)

- **Goal:** Debug topology, optimize performance, manage firmware.
- **Experience:** Hierarchical tree view (Host → Bridges → Devices).
- **Data:** Exposes PCI paths, driver names, link speeds, NVM versions, IRQs.
- **Control:** Access to advanced features (Manual Firmware Flash, Force
  De-auth) protected by **High Friction** workflows.

## 5. Implementation Rules

1.  **Never Brick:** Any operation that writes to non-volatile memory (NVM)
    must verify the target is not critical for the current session (e.g.,
    active display, boot drive). If critical, **block** the operation.
2.  **Read Before Write:** Before suggesting a change (BIOS setting, cable
    move), verify the current state via `sysfs`. Do not guess.
3.  **Atomic Operations:** Backend operations involving state changes (enroll,
    forget, flash) must be atomic. If a step fails, roll back or report a
    precise error code (e.g., parsing `nvm_authenticate` errors).
4.  **No Silent Failures:** If a device disappears during an operation,
    distinguish between "Expected Reset" (firmware flash) and "Unexpected
    Error" (cable fault).
5.  **Documentation Driven:** Every feature must be traceable back to kernel
    documentation or `boltd` specs. We do not implement "magic" heuristics; we
    implement documented behavior.

## 6. Conclusion

**Cosmic Ext Thunderbolt** treats the user as an intelligent partner, not a
passive recipient. By combining the raw power of the Linux kernel with a
thoughtful, safety-first interface, we turn the complexity of Thunderbolt from
a source of fear into a tool for empowerment.

> _"The interface presented here is not meant for end users. Instead there
> should be a userspace tool that handles all the low-level details..."_
> — Linux Kernel Thunderbolt Documentation

We ambition to be that tool.
