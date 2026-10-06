"""Translate PyBoy's four sound channels, preserving their arithmetic and dispatch."""

import ast
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "reference/pyboy-2.7.0/pyboy/core/sound.py"
OPS = {ast.Add: "+", ast.Sub: "-", ast.Mult: "*", ast.FloorDiv: "/",
       ast.Mod: "%", ast.BitAnd: "&", ast.BitOr: "|", ast.BitXor: "^",
       ast.LShift: "<<", ast.RShift: ">>", ast.Eq: "==", ast.NotEq: "!=",
       ast.Lt: "<", ast.Gt: ">", ast.LtE: "<=", ast.GtE: ">="}


def expr(node):
    if isinstance(node, ast.Constant):
        return str(int(node.value))
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return ast.unparse(node).lower()
    if isinstance(node, (ast.List, ast.Tuple)):
        return "vec![" + ", ".join(map(expr, node.elts)) + "]"
    if isinstance(node, ast.Subscript):
        return f"{expr(node.value)}[({expr(node.slice)}) as usize]"
    if isinstance(node, ast.UnaryOp):
        if isinstance(node.op, ast.Not):
            return f"i64::from({expr(node.operand)} == 0)"
        operator = {ast.Invert: "!", ast.USub: "-"}[type(node.op)]
        return f"({operator}{expr(node.operand)})"
    if isinstance(node, ast.BinOp):
        if isinstance(node.op, ast.Mult) and isinstance(node.left, ast.List):
            assert len(node.left.elts) == 1
            return f"vec![{expr(node.left.elts[0])}; {expr(node.right)}]"
        return f"({expr(node.left)} {OPS[type(node.op)]} {expr(node.right)})"
    if isinstance(node, ast.Compare):
        assert len(node.ops) == 1
        return f"i64::from({expr(node.left)} {OPS[type(node.ops[0])]} {expr(node.comparators[0])})"
    if isinstance(node, ast.BoolOp):
        first = expr(node.values[0])
        rest = expr(node.values[1]) if len(node.values) == 2 else expr(ast.BoolOp(op=node.op, values=node.values[1:]))
        yes, no = (first, rest) if isinstance(node.op, ast.Or) else (rest, first)
        return f"(if {first} != 0 {{ {yes} }} else {{ {no} }})"
    if isinstance(node, ast.IfExp):
        return f"(if {expr(node.test)} != 0 {{ {expr(node.body)} }} else {{ {expr(node.orelse)} }})"
    if isinstance(node, ast.Call):
        name = ast.unparse(node.func)
        if name.startswith("logger."):
            return "0"
        if name == "array":
            return expr(node.args[1])
        if name.startswith("ToneChannel."):
            name = "self.tone_" + name.split(".")[1]
            args = node.args[1:]
        else:
            args = node.args
        return name.lower() + "(" + ", ".join(map(expr, args)) + ")"
    raise ValueError(ast.dump(node))


def statements(nodes, declared):
    out = []
    for node in nodes:
        if isinstance(node, ast.Assign):
            value = expr(node.value)
            for target in node.targets:
                if ast.unparse(target) == "self.sweepenable":
                    value = f"i64::from(({value}) != 0)"
                prefix = ""
                if isinstance(target, ast.Name) and target.id not in declared:
                    prefix = "let mut "
                    declared.add(target.id)
                out.append(f"{prefix}{expr(target)} = {value};")
        elif isinstance(node, ast.AugAssign):
            out.append(f"{expr(node.target)} {OPS[type(node.op)]}= {expr(node.value)};")
        elif isinstance(node, ast.If):
            out.append(f"if {expr(node.test)} != 0 {{")
            out.extend(statements(node.body, declared.copy()))
            if node.orelse:
                out.append("} else {")
                out.extend(statements(node.orelse, declared.copy()))
            out.append("}")
        elif isinstance(node, ast.While):
            out.append(f"while {expr(node.test)} != 0 {{")
            out.extend(statements(node.body, declared.copy()))
            out.append("}")
        elif isinstance(node, ast.Return):
            out.append("return " + (expr(node.value) if node.value else "0") + ";")
        elif isinstance(node, ast.Expr):
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                continue
            if isinstance(node.value, ast.Call) and ast.unparse(node.value.func).startswith("logger."):
                continue
            out.append(expr(node.value) + ";")
        elif isinstance(node, ast.Pass):
            pass
        else:
            raise ValueError(ast.dump(node))
    return out


def fields(init):
    result = {}
    for node in init.body:
        if not isinstance(node, ast.Assign):
            continue
        target = node.targets[0]
        assert isinstance(target, ast.Attribute) and ast.unparse(target.value) == "self"
        value = expr(node.value)
        kind = "i64"
        if value.startswith("vec!["):
            kind = "Vec<Vec<i64>>" if value.startswith("vec![vec![") else "Vec<i64>"
        result[target.attr.lower()] = (kind, value)
    return result


def state_fields(nodes, methods):
    result = []
    for node in nodes:
        if isinstance(node, ast.For):
            count = ast.literal_eval(node.iter.args[0])
            for index in range(count):
                for field, width in state_fields(node.body, methods):
                    result.append((field.replace("[(n) as usize]", f"[{index}]"), width))
        elif isinstance(node, ast.Expr) and isinstance(node.value, ast.Call):
            call = node.value
            name = ast.unparse(call.func)
            if name == "ToneChannel.save_state":
                result.extend(state_fields(methods["ToneChannel"]["save_state"].body, methods))
            else:
                width = {"file.write": 1, "file.write_16bit": 2, "file.write_64bit": 8}[name]
                result.append((expr(call.args[0]), width))
        else:
            raise ValueError(ast.dump(node))
    return result


def main():
    manifest = json.loads((ROOT / "reference/manifest.json").read_text())
    assert hashlib.sha256(SOURCE.read_bytes()).hexdigest() == manifest["files"]["pyboy/core/sound.py"]
    classes = {n.name: n for n in ast.parse(SOURCE.read_text()).body if isinstance(n, ast.ClassDef)}
    methods = {name: {m.name: m for m in cls.body if isinstance(m, ast.FunctionDef)} for name, cls in classes.items()}
    out = ["// Generated from PyBoy 2.7.0 core/sound.py by tools/generate_channels.py.",
           "// SPDX-License-Identifier: LGPL-3.0-only",
           "#![allow(unused_parens, unused_mut, unused_assignments, unreachable_code, clippy::all)]"]
    for name in ("ToneChannel", "SweepChannel", "WaveChannel", "NoiseChannel"):
        funcs = dict(methods[name])
        attrs = fields(funcs["__init__"])
        if name == "SweepChannel":
            attrs = fields(methods["ToneChannel"]["__init__"]) | attrs
            funcs = methods["ToneChannel"] | funcs
            funcs |= {"tone_" + n: m for n, m in methods["ToneChannel"].items() if n not in {"__init__", "save_state", "load_state"}}
        out.extend(["#[derive(Debug, Clone)]", f"pub struct {name} {{"])
        out.extend(f"pub {key}: {kind}," for key, (kind, _) in attrs.items())
        out.extend(["}", f"impl {name} {{"])
        args = "cgb: i64" if name == "WaveChannel" else ""
        out.extend([f"pub fn new({args}) -> Self {{", "Self {"])
        out.extend(f"{key}: {value}," for key, (_, value) in attrs.items())
        out.extend(["}", "}"])
        for fname, method in funcs.items():
            if fname in {"__init__", "save_state", "load_state"}:
                continue
            args = [arg.arg for arg in method.args.args[1:]]
            signature = ", ".join(["&mut self"] + [arg + ": i64" for arg in args])
            out.append(f"pub fn {fname}({signature}) -> i64 {{")
            locals_used = {n.id for n in ast.walk(method) if isinstance(n, ast.Name) and isinstance(n.ctx, ast.Store)}
            out.extend(f"let mut {local}: i64 = 0;" for local in sorted(locals_used))
            out.extend(statements(method.body, set(args) | locals_used))
            out.extend(["0", "}"])
        out.append("pub fn codec(&mut self, codec: &mut crate::state::Codec) -> Result<(), &'static str> {")
        for field, width in state_fields(methods[name]["save_state"].body, methods):
            out.append(f"codec.int(&mut {field}, {width})?;")
        out.extend(["Ok(())", "}"])
        out.append("}")
    target = ROOT / "crates/pyboy-core/src/channels.rs"
    target.write_text("\n".join(out) + "\n")
    subprocess.run(["rustfmt", "--edition", "2024", str(target)], check=True)


if __name__ == "__main__":
    main()
