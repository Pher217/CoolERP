"""Apply the reviewed overlay when packaging; raw generation stays untouched."""
import importlib.util
from pathlib import Path

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version, build_data):
        if self.target_name == "wheel":
            path = Path(self.root) / "prepare_sdk.py"
            spec = importlib.util.spec_from_file_location("coolerp_prepare", path)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            module.prepare(Path(self.root))
