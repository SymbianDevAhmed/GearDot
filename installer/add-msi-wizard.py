"""Add the shared GearDot setup wizard to a WiX or wixl package."""

import argparse
import os
import subprocess
import tempfile
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("msi", type=Path)
parser.add_argument("--msibuild", default="msibuild")
parser.add_argument("--msiinfo", default="msiinfo")
options = parser.parse_args()
msi = options.msi.resolve()
work = tempfile.TemporaryDirectory()
folder = Path(work.name)
dialogs, controls, events, conditions, mappings = [], [], [], [], []


def table(name, columns, types, keys, rows):
    output = [columns.split(), types.split(), [name, *keys.split()], *rows]
    data = "\r\n".join("\t".join(map(str, row)) for row in output) + "\r\n"
    path = folder / f"{name}.idt"
    path.write_bytes(data.encode("ascii"))
    return str(path)


def event(dialog, control, name, argument, order=1, condition="1"):
    events.append([dialog, control, name, argument, condition, order])


def dialog(name, heading, message, buttons, *, extra=(), cancel="Cancel"):
    buttons = list(buttons)
    button_controls = [
        (ident, "PushButton", x, 194, width, 20, 3, "", label)
        for ident, label, x, width in buttons
    ]
    active = [item[0] for item in extra if item[1] not in ("Text", "ProgressBar")]
    active += [item[0] for item in button_controls]
    first_button = button_controls[0][0] if button_controls else ""
    dialogs.append([name, 50, 50, 380, 228, 3, "GearDot Setup",
                    active[0] if active else "", first_button, cancel])
    layout = (
        ("Heading", "Text", 18, 20, 344, 24, 3, "", "{\\Title}" + heading),
        ("Message", "Text", 18, 58, 344, 33, 3, "", message),
        ("FooterRule", "Line", 0, 185, 380, 0, 1, "", ""),
        *extra, *button_controls,
    )
    for item in layout:
        ident = item[0]
        following = active[(active.index(ident) + 1) % len(active)] if ident in active else ""
        controls.append([name, *item, following, ""])


cancel = ("Cancel", "Cancel", 302, 60)
back = ("Back", "< Back", 164, 64)
next_button = ("Next", "Next >", 230, 64)
finish = ("Finish", "Finish", 302, 60)

dialog("Welcome", "Welcome to GearDot Setup",
       "Setup will guide you through the installation of GearDot.",
       (("Install", "Next >", 230, 64), cancel))
event("Welcome", "Install", "NewDialog", "InstallFolder")
event("Welcome", "Cancel", "SpawnDialog", "ConfirmCancel")

dialog("InstallFolder", "Choose where to install GearDot",
       "Use the suggested location, enter another folder or choose Browse.",
       (next_button, back, cancel), extra=(
           ("Folder", "PathEdit", 18, 98, 270, 18, 3, "INSTALLFOLDER", ""),
           ("Browse", "PushButton", 300, 98, 62, 18, 3, "", "Browse..."),
       ))
event("InstallFolder", "Next", "SetTargetPath", "INSTALLFOLDER", 1)
event("InstallFolder", "Next", "NewDialog", "ReadyToInstall", 2)
event("InstallFolder", "Back", "NewDialog", "Welcome")
event("InstallFolder", "Browse", "SpawnDialog", "BrowseFolder")
event("InstallFolder", "Cancel", "SpawnDialog", "ConfirmCancel")

dialog("BrowseFolder", "Browse for a folder", "Choose a location for GearDot.",
       (("Accept", "OK", 230, 64), cancel), extra=(
           ("Drive", "DirectoryCombo", 18, 85, 214, 80, 0x30003, "INSTALLFOLDER", ""),
           ("Up", "PushButton", 240, 85, 55, 18, 3, "", "Up"),
           ("New", "PushButton", 300, 85, 62, 18, 3, "", "New folder"),
           ("List", "DirectoryList", 18, 108, 344, 48, 7, "INSTALLFOLDER", ""),
           ("Path", "PathEdit", 18, 161, 344, 16, 3, "INSTALLFOLDER", ""),
       ))
event("BrowseFolder", "Up", "DirectoryListUp", "0")
event("BrowseFolder", "New", "DirectoryListNew", "0")
event("BrowseFolder", "Accept", "SetTargetPath", "INSTALLFOLDER", 1)
event("BrowseFolder", "Accept", "EndDialog", "Return", 2)
event("BrowseFolder", "Cancel", "Reset", "0", 1)
event("BrowseFolder", "Cancel", "EndDialog", "Return", 2)
mappings.append(["BrowseFolder", "Drive", "IgnoreChange", "IgnoreChange"])

dialog("ReadyToInstall", "Ready to install", "GearDot will be installed in [INSTALLFOLDER]",
       (("Install", "Install", 230, 64), back, cancel))
event("ReadyToInstall", "Back", "NewDialog", "InstallFolder")
event("ReadyToInstall", "Install", "EndDialog", "Return")
event("ReadyToInstall", "Cancel", "SpawnDialog", "ConfirmCancel")

dialog("Maintenance", "Manage your GearDot installation",
       "GearDot is already installed on this PC.",
       (cancel,), extra=(
           ("Repair", "PushButton", 18, 95, 78, 24, 3, "", "Repair"),
           ("RepairDescription", "Text", 107, 101, 255, 18, 3, "", "Restore missing or damaged program files."),
           ("Remove", "PushButton", 18, 133, 78, 24, 3, "", "Uninstall"),
           ("RemoveDescription", "Text", 107, 139, 255, 18, 3, "", "Remove GearDot and its shortcuts."),
       ))
dialogs[-1][8] = "Repair"
event("Maintenance", "Repair", "NewDialog", "ConfirmRepair")
event("Maintenance", "Remove", "NewDialog", "ConfirmRemove")
event("Maintenance", "Cancel", "SpawnDialog", "ConfirmCancel")
dialog("ConfirmRepair", "Repair GearDot", "Setup will restore the program files and shortcuts.",
       (("Repair", "Repair", 230, 64), back, cancel))
event("ConfirmRepair", "Repair", "ReinstallMode", "amus", 1)
event("ConfirmRepair", "Repair", "Reinstall", "All", 2)
event("ConfirmRepair", "Repair", "EndDialog", "Return", 3)
dialog("ConfirmRemove", "Uninstall GearDot", "Setup will remove the program and its shortcuts.",
       (("Remove", "Uninstall", 230, 64), back, cancel))
event("ConfirmRemove", "Remove", "Remove", "All", 1)
event("ConfirmRemove", "Remove", "EndDialog", "Return", 2)
for name in ("ConfirmRepair", "ConfirmRemove"):
    event(name, "Back", "NewDialog", "Maintenance")
    event(name, "Cancel", "SpawnDialog", "ConfirmCancel")

dialog("ConfirmCancel", "Cancel setup?", "Do you want to close GearDot Setup?",
       (("Continue", "Continue setup", 188, 108), ("Exit", "Exit setup", 302, 60)),
       cancel="Continue")
event("ConfirmCancel", "Continue", "EndDialog", "Return")
event("ConfirmCancel", "Exit", "EndDialog", "Exit")

dialog("Progress", "Setup in progress", "Please wait while Windows finishes the requested changes.", (cancel,),
       extra=(("Bar", "ProgressBar", 18, 105, 344, 12, 3, "", ""),))
dialogs[-1][5] = 1  # The installer displays this dialog while executing the script.
mappings.append(["Progress", "Bar", "SetProgress", "Progress"])
event("Progress", "Cancel", "SpawnDialog", "ConfirmCancel")

dialog("Completed", "Setup complete", "GearDot Setup finished successfully.", (finish,),
       extra=(("Launch", "CheckBox", 18, 104, 160, 18, 3, "LAUNCHGEARDOT", "Launch GearDot"),),
       cancel="Finish")
conditions += [["Completed", "Launch", "Hide", "Installed"],
               ["Completed", "Launch", "Show", "NOT Installed"]]
event("Completed", "Finish", "DoAction", "LaunchGearDot", 1,
      "NOT Installed AND LAUNCHGEARDOT = 1")
event("Completed", "Finish", "EndDialog", "Return", 2)
dialog("SetupFailed", "Setup could not finish",
       "Windows Installer could not complete the operation.", (finish,), cancel="Finish")
event("SetupFailed", "Finish", "EndDialog", "Exit")
dialog("SetupCancelled", "Setup cancelled", "GearDot Setup was cancelled.",
       (finish,), cancel="Finish")
event("SetupCancelled", "Finish", "EndDialog", "Exit")

windows_editor = Path(__file__).with_name("edit-msi.ps1")
if os.name == "nt":
    subprocess.run(["powershell.exe", "-NoProfile", "-File", windows_editor,
                    "-Package", msi, "-Directory", folder, "-ExportSequence"], check=True)
    sequence = (folder / "InstallUISequence.idt").read_text(encoding="ascii")
else:
    sequence = subprocess.check_output([options.msiinfo, "export", msi, "InstallUISequence"], text=True)
sequence = [line.split("\t") for line in sequence.splitlines()[3:] if line]
sequence += [["Welcome", 'NOT Installed', 1240],
             ["Maintenance", 'Installed AND NOT RESUME AND NOT Preselected AND NOT REMOVE AND NOT REINSTALL', 1241],
             ["Progress", "", 1280], ["Completed", "", -1],
             ["SetupFailed", "", -3], ["SetupCancelled", "", -2]]

tables = [
    table("Dialog", "Dialog HCentering VCentering Width Height Attributes Title Control_First Control_Default Control_Cancel",
          "s72 i2 i2 i2 i2 I4 L128 S50 S50 S50", "Dialog", dialogs),
    table("Control", "Dialog_ Control Type X Y Width Height Attributes Property Text Control_Next Help",
          "s72 s50 s20 i2 i2 i2 i2 I4 S50 L0 S50 L50", "Dialog_ Control", controls),
    table("ControlEvent", "Dialog_ Control_ Event Argument Condition Ordering",
          "s72 s50 s50 l255 S255 I2", "Dialog_ Control_ Event Argument Condition", events),
    table("ControlCondition", "Dialog_ Control_ Action Condition",
          "s72 s50 s50 s255", "Dialog_ Control_ Action Condition", conditions),
    table("EventMapping", "Dialog_ Control_ Event Attribute",
          "s72 s50 s50 s50", "Dialog_ Control_ Event", mappings),
    table("CheckBox", "Property Value", "s72 l64", "Property", [["LAUNCHGEARDOT", 1]]),
    table("TextStyle", "TextStyle FaceName Size Color StyleBits",
          "s72 l32 i2 I4 I2", "TextStyle", [
              ["Normal", "Segoe UI", 9, "", 0],
              ["Title", "Segoe UI", 15, "", 1],
          ]),
    table("InstallUISequence", "Action Condition Sequence", "s72 S255 I2", "Action", sequence),
]
if os.name == "nt":
    subprocess.run(["powershell.exe", "-NoProfile", "-File", windows_editor,
                    "-Package", msi, "-Directory", folder], check=True)
    raise SystemExit

args = [options.msibuild, msi]
for path in tables:
    args += ["-i", path]
args += ["-q", "UPDATE `Property` SET `Value` = 'INSTALLFOLDER;WIX_DOWNGRADE_DETECTED;WIX_UPGRADE_DETECTED' "
               "WHERE `Property` = 'SecureCustomProperties'",
         "-q", "UPDATE `InstallExecuteSequence` SET `Sequence` = 1501 "
               "WHERE `Action` = 'RemoveExistingProducts'",
         "-q", "UPDATE `Upgrade` SET `Attributes` = 513 "
               "WHERE `ActionProperty` = 'WIX_UPGRADE_DETECTED'"]
subprocess.run(args, cwd=folder, check=True)
