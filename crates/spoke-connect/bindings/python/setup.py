"""Platform wheel tags for bundled native FFI (py3-none-<platform>, no sdist)."""

import os

from setuptools import setup
from setuptools.command.bdist_wheel import bdist_wheel

RID_TO_PLAT = {
    "linux-x64": "manylinux_2_17_x86_64",
    "win-x64": "win_amd64",
    "osx-arm64": "macosx_11_0_arm64",
}


class PlatformBdistWheel(bdist_wheel):
    """Emit py3-none-<platform> wheels with Root-Is-Purelib: false."""

    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False
        rid = os.environ.get("SPOKE_PYTHON_WHEEL_RID", "osx-arm64")
        plat = RID_TO_PLAT.get(rid)
        if plat is None:
            msg = f"unsupported SPOKE_PYTHON_WHEEL_RID={rid!r}"
            raise SystemExit(msg)
        self.plat_name = plat
        self.plat_name_supplied = True

    def get_tag(self) -> tuple[str, str, str]:
        _python, _abi, plat = super().get_tag()
        return "py3", "none", plat


setup(cmdclass={"bdist_wheel": PlatformBdistWheel})
