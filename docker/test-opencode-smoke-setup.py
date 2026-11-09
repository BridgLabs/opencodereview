#!/usr/bin/env python3
import ast
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest


def tree(name):
    return ast.parse(Path(__file__).with_name(name).read_text())


def execute(nodes, namespace):
    exec(compile(ast.Module(body=nodes, type_ignores=[]), __file__, "exec"), namespace)


class SmokeSetupTests(unittest.TestCase):
    def test_missing_parent_and_existing_permissions(self):
        for name in ["opencode-smoke.py", "opencode-runner-smoke.py"]:
            with self.subTest(script=name), tempfile.TemporaryDirectory() as directory:
                parent = Path(directory) / "opencode"
                source = tree(name)
                setup = next(node for node in ast.walk(source) if isinstance(node, ast.Expr)
                             and isinstance(node.value, ast.Call)
                             and isinstance(node.value.func, ast.Attribute)
                             and node.value.func.attr == "mkdir"
                             and ast.unparse(node.value.func.value) == "Path('/tmp/opencode')")
                temporary = next(node for node in ast.walk(source) if isinstance(node, ast.With)
                                 and "TemporaryDirectory" in ast.unparse(node.items[0].context_expr))
                self.assertLess(setup.lineno, temporary.lineno)
                namespace = {"Path": lambda value: parent, "tempfile": tempfile, "prefix": "proof-"}
                execute([setup], namespace)
                self.assertEqual(parent.stat().st_mode & 0o777, 0o700)
                context = temporary.items[0].context_expr
                for keyword in context.keywords:
                    if keyword.arg == "dir":
                        keyword.value = ast.Constant(str(parent))
                with eval(compile(ast.fix_missing_locations(ast.Expression(context)), __file__, "eval"), namespace) as root:
                    self.assertEqual(Path(root).parent, parent)
                    self.assertEqual(Path(root).stat().st_mode & 0o777, 0o700)
                parent.chmod(0o750)
                execute([setup], namespace)
                self.assertEqual(parent.stat().st_mode & 0o777, 0o750)

    def test_provider_identity_preserves_isolation(self):
        source = tree("opencode-runner-smoke.py")
        identity = next(node for node in ast.walk(source) if isinstance(node, ast.Assign)
                        and any(isinstance(target, ast.Name) and target.id == "provider_user" for target in node.targets))
        launch = next(node for node in ast.walk(source) if isinstance(node, ast.Expr)
                      and isinstance(node.value, ast.Call) and ast.unparse(node.value.func) == "engine"
                      and any(isinstance(arg, ast.Constant) and arg.value == "/provider/proof.py" for arg in node.value.args))
        for engine in ["docker", "podman"]:
            with self.subTest(engine=engine):
                calls = []
                namespace = {"os": os, "args": SimpleNamespace(engine=engine, provider_image="local-python"),
                             "engine": lambda *args: calls.append(args), "provider": "private-api",
                             "root": Path("/private"), "volume_mode": "rw,Z" if engine == "podman" else "rw"}
                execute([identity, launch], namespace)
                command, = calls
                for flag in ["--pull=never", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges:true"]:
                    self.assertIn(flag, command)
                if engine == "docker":
                    self.assertEqual(command[command.index("--user") + 1], f"{os.getuid()}:{os.getgid()}")
                else:
                    self.assertNotIn("--user", command)


if __name__ == "__main__":
    unittest.main()
