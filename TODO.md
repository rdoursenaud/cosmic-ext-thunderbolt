# Thunderbolt™ for the COSMIC™ DE - TODO list

## Status: Public Alpha (v0.0.1)
The core functionality (Device listing, Authorization, Enrollment, Forget)
is implemented and stable.
Strengthening, basic configuration page are planned for subsequent alphas.
Advanced features (IOMMU detection, Firmware updates, Topology view)
are planned for the beta (v0.1).

## Backend (Common)

- [x] Uses boltd D-Bus interface for interrogation, domain workflows
  - [ ] Logical topology
    - [x] Flat
    - [ ] Tree
  - [ ] Device authentication lifecycle
    - [ ] Authorize
    - [x] Enroll
    - [x] Forget
- [ ] Uses kernel PCI subsystem through Udev to add contextual information
  - [ ] Physical topology
    - [ ] Bridges/Docks/Adapters
  - [ ] Loaded module(s)
- [ ] Mini Log
  - [ ] 10 latest events
- [ ] Power status
  - [ ] Recommendations if resources available/limited
- [ ] Bandwidth waterfall
  - [ ] Graph with allocated portions
  - [ ] Recommendations if resources available/limited
- [ ] Security analysis
  - [ ] Detect IOMMU DMA Protection (`/sys/.../iommu_dma_protection`)
  - [ ] Differentiate modes:
    - [ ] Hardware Isolated (IOMMU) -> Highest Trust, No BIOS change needed.
    - [x] Cryptographic (Secure Connect) -> High Trust.
    - [x] Software ACL (User) -> Medium Trust.
    - [x] Legacy (None) -> Low Trust.
  - [ ] Recommendations:
    - [ ] If IOMMU active: Inform user "Hardware Protected".
      Suppress "Upgrade to Secure" advice.
    - [ ] If IOMMU inactive on recent HW: Suggest enabling VT-d/IOMMU in
      BIOS/Kernel.
- [ ] Firmware Management (via fwupd D-Bus)
  - [ ] List devices (Host, Retimer, End-devices)
  - [ ] Check versions (Current vs Latest)
  - [ ] Detect critical updates (Bugfixes/Security)
  - [ ] Trigger updates
  - [ ] Monitor progress (Writing, Rebooting, Success)
- [ ] Tunnel Event Listener (Real-time Bandwidth)
  - [ ] Listen for KOBJ_CHANGE on subsystem 'thunderbolt_domain'.
  - [ ] Parse environment variables:
    - [ ] TUNNEL_EVENT (activated, changed, deactivated, low/insufficient
      bandwidth).
    - [ ] TUNNEL_DETAILS (Source/Dest Ports, Type).
  - [ ] Dynamic Waterfall UI:
    - [ ] Visualize allocated bandwidth per tunnel in real-time.
    - [ ] Animate changes when events are received.
  - [ ] Alerting:
    - [ ] Immediate notification on "insufficient bandwidth".
    - [ ] Contextual advice on which device to move/unplug.
  - [ ] Fallback: Handle cases where TUNNEL_DETAILS is missing (Firmware CM).
- [ ] Thunderbolt Networking Support
  - [ ] Detect `thunderbolt_net` module and `thunderboltX` interfaces.
  - [ ] Correlate network interface to the remote Host Device in the topology
    tree.
  - [ ] Auto-Load Logic:
    - [ ] If cable connected between two Linux hosts and no interface exists:
      Prompt to `modprobe thunderbolt-net`.
  - [ ] Configuration Wizard:
    - [ ] Offer quick setup (Link-Local IP or Static IP suggestion).
    - [ ] Warn about Firewall rules on this high-speed interface.
  - [ ] Visual Indicator:
    - [ ] Show "Host-to-Host Link" icon with current throughput capability
      (10/20/40 Gbps).
    - [ ] Display real-time network stats (RX/TX) if possible.
  - [ ] Security Warning:
    - [ ] Remind user that this opens a PCIe tunnel; ensure remote host is
      trusted.
- [ ] Thunderbolt Stream Manager (USB4STREAM)
  - [ ] Monitor ConfigFS: `/sys/kernel/config/thunderbolt/stream/`.
  - [ ] Wizard: Create named streams (e.g., "data", "backup").
    - [ ] Auto-configure HopIDs (-1).
    - [ ] Handle XDomain negotiation automatically.
  - [ ] Device Exposure:
    - [ ] Map created streams to `/dev/tbstreamX`.
    - [ ] Provide quick actions: "Test Throughput", "Open Terminal", "Create
      Udev Rule".
  - [ ] Multi-Stream Support:
    - [ ] Visualize multiple concurrent streams + thunderbolt-net on the same
      link.
  - [ ] Use-Case Templates:
    - [ ] "Fast Backup" (dd preset).
    - [ ] "Low-Latency IPC" (cluster preset).
  - [ ] Security:
    - [ ] Warn about raw DMA-like access to remote host.
    - [ ] Manage permissions for /dev/tbstreamX.
- [ ] Force Power Management (WMI)
  - [ ] Detect `force_power` attribute via WMI/Intel platform sysfs.
  - [ ] State Handling:
    - [ ] Since state cannot be queried, use "Action-Based" UI (Toggle On/Off)
      rather than status indicator.
    - [ ] Visual feedback: Distinct "Maintenance Mode" theme when active.
  - [ ] Automation:
    - [ ] Integrate into Firmware Update Wizard:
      Auto-enable before flash (if no cable), auto-disable after.
  - [ ] Safety:
    - [ ] Warn about increased power consumption and potential sleep prevention.
    - [ ] Optional: Auto-off timer (e.g., 5 mins) to prevent battery drain.
  - [ ] Fallback: If attribute missing, inform user "Platform does not support
    force power; cable required for operations."
- [ ] UCSI / Type-C Power Management
  - [ ] Monitor `/sys/class/typec/` for port status.
  - [ ] Correlate Type-C ports with Thunderbolt domains.
  - [ ] Display Power Delivery info:
    - [ ] Current Role (Source/Sink).
    - [ ] Negotiated Voltage/Current (Watts).
    - [ ] Partner capabilities (if available).
  - [ ] Expert Feature: Allow role swap (Source <-> Sink) via `try_role` (with
    Polkit auth).
  - [ ] Fallback: Gracefully handle systems without UCSI support.

## Applet for cosmic-panel

- [x] Main icon
  - [x] Represents state
  - [ ] Alerts on insecure settings (exclamation mark)
  - [x] Alerts on available actions (dot)
  - [ ] Combines both on actionnable errors (dot + exclamation)
- Upon opening:
  - [x] Show current security level if problematic (Unknown, None, DpOnly,
        USBOnly)
  - [x] If security level is good (User, Secure)
    - [x] List devices (Flat)
      - [ ] Actionnables first
        - [ ] Awaiting Authentication
          - [ ] Actions: Authorize (temporary session)
        - [ ] Authorized but not enrolled (temporary)
          - [x] Action: Enroll (stored)
      - [ ] Informative in a sub-menu with a useful title (eg "Known devices
        (Connected 4/6) ...")
        - [x] Connected (Enrolled)
          - [x] Action: Forget (needs disconnect/restart)
        - [x] Enrolled Disconnected
          - [x] Action: Forget (Immediate)
    - [x] Button to access configuration page

## Configuration Page for cosmic-settings

- [ ] Search feature
- [ ] Hierarchical view
  - [ ] Global settings/status
    - [ ] Domain settings/status
      - [ ] Device(s) settings/status
- [ ] Detailed views
  - [ ] PCI path
  - [ ] IDs
  - [ ] Link-speed
  - [ ] Loaded module(s)
  - [ ] NVM versions
- [ ] Immediate deauthorization
  - [ ] With the proper amount of friction due to the data loss risks
- [ ] Mini Log display
- [ ] Power status
  - [ ] Recommendations if resources available/limited
- [ ] Bandwidth waterfall
  - [ ] Graph with allocated portions
  - [ ] Recommendations if resources available/limited
- [ ] Firmware Section (per device)
  - [ ] Display Current Version & Latest Available
  - [ ] "Update" button (with safety friction: battery check, warnings)
  - [ ] Update History / Changelog (from LVFS metadata)
  - [ ] Status indicator during update (Progress bar, "Do not disconnect")
- [ ] Manual Firmware Update (Expert Only)
  - [ ] Safety Checks (CRITICAL):
    - [ ] **NO UPDATE IF ACTIVE DISPLAY**: Detect if the target Thunderbolt
      device provides the current display output (eGPU scenario).
      - If YES: BLOCK manual update immediately. Display strict warning to
        switch to integrated graphics or remote session first.
      - Rationale: Preventing "self-bricking" by losing GUI during the
        flash/reboot cycle.
    - [ ] Check if device is a boot drive (root filesystem). If YES, warn
      strongly or block (though less critical than GPU as kernel handles I/O
      errors, still risky).
  - [ ] File picker for binary images (`.bin`, `.img`, others?).
  - [ ] Pre-flight checks:
    - [ ] Verify file size and basic header integrity.
    - [ ] Ensure target device has `nvm_non_active*` slot.
    - [ ] Check battery status (AC required).
  - [ ] Secure Write Process:
    - [ ] Write to `nvm_non_activeN/nvmem` (NEVER active).
    - [ ] Trigger `nvm_authenticate`.
    - [ ] Parse error codes from `nvm_authenticate` (0x0 = Success).
  - [ ] UI Friction:
    - [ ] Hidden behind "Advanced/Debug" menu.
    - [ ] Explicit warnings about "Bricking" risks.
    - [ ] Mandatory confirmation checkbox.
    - [ ] No cancellation once started.
- [ ] Retimer Firmware Update (No Cable Mode)
  - [ ] Detect `offline` and `rescan` attributes on usb4_port devices.
  - [ ] Pre-flight checks:
    - [ ] Ensure port is physically empty (no device connected).
    - [ ] Ensure port is NOT driving current session (display/input).
  - [ ] Automated Sequence (Atomic):
    1. Set `offline = 1`.
    2. Trigger `rescan` to enumerate retimer.
    3. Flash retimer NVM (via fwupd or manual .bin).
    4. Trigger `nvm_authenticate`.
    5. Wait > 5 seconds (Critical!).
    6. Trigger `rescan` again.
    7. Set `offline = 0` to restore port.
  - [ ] UI: Show strict progress bar. Block interaction during the process.
  - [ ] Error Handling: Auto-recover to `online` state if possible on failure.
- [ ] Recovery Mode (Safe Mode Detection)
  - [ ] Detect "Safe Mode": `nvm_version` returns ENODATA,
    missing identity info.
  - [ ] UI: Critical Alert Banner ("Controller Bricked").
  - [ ] Wizard:
    - [ ] Guide user to download correct NVM image
      (based on motherboard model via dmidecode).
    - [ ] Manual file selection (no auto-match possible).
    - [ ] Pre-flight: Verify binary header matches silicon ID
      (read raw PCI config if needed).
  - [ ] Flash Process:
    - [ ] Write to `nvm_non_active`.
    - [ ] Authenticate.
  - [ ] Post-Flash Instruction:
    - [ ] Explicitly demand a **Full Power Cycle** (Shutdown + Unplug),
      not just Reboot.
    - [ ] Explain why:
      "Controller needs cold boot to load new NVM from safe mode."
