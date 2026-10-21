"""Writes tests/fixtures/reference.json: the vectors onnxruntime computes on the
pinned ONNX export, which `vectors_match_the_onnx_reference` compares with.

The reference is independent of everything `oxyn-embed` does: another runtime,
the Python tokenizer, one text per run so that no padding is involved. The
pipeline is the published one — CLS pooling, then L2 normalization — with the
same 512-token truncation.

    uv run --no-project --with onnxruntime==1.30.0 --with numpy==2.5.3 \\
        --with tokenizers==0.23.2 python -I crates/oxyn-embed/codegen/reference.py \\
        <model.onnx> <tokenizer.json> crates/oxyn-embed/tests/fixtures/reference.json

`<model.onnx>` is the pinned export `regenerate.py` downloads; `<tokenizer.json>`
the pinned tokenizer of `src/pinned.rs`.
"""

import json
import sys

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer

MAX_TOKENS = 512

# What the ranking will see: questions in two languages, catalog names in
# their usual shapes, and the inputs a server can send — empty, very long,
# non-Latin, with SQL in the name.
TEXTS = [
    "Which customers have unpaid invoices?",
    "Quels clients ont des factures impayées ?",
    "customer_invoices",
    "warehouse_stock_movements",
    "user_login_audit",
    "public.order_line_items",
    "",
    '"users"; DROP TABLE audit; --',
    "顧客の請求書",
    "Zahlungsrückstände pro Kunde 💶",
    "invoice " * 700,
]


def main() -> None:
    onnx_path, tokenizer_path, out_path = sys.argv[1:4]
    tokenizer = Tokenizer.from_file(tokenizer_path)
    tokenizer.no_padding()
    tokenizer.enable_truncation(max_length=MAX_TOKENS)
    session = ort.InferenceSession(onnx_path, providers=["CPUExecutionProvider"])
    vectors = []
    for text in TEXTS:
        encoding = tokenizer.encode(text)
        ids = np.array([encoding.ids], dtype=np.int64)
        mask = np.array([encoding.attention_mask], dtype=np.int64)
        hidden = session.run(None, {"input_ids": ids, "attention_mask": mask})[0]
        cls = hidden[0, 0, :]
        unit = cls / np.linalg.norm(cls)
        vectors.append([round(float(v), 7) for v in unit])
    with open(out_path, "w", encoding="utf-8") as out:
        json.dump(
            {
                "producer": f"onnxruntime {ort.__version__}, tokenizers, codegen/reference.py",
                "texts": TEXTS,
                "vectors": vectors,
            },
            out,
            ensure_ascii=False,
        )
        out.write("\n")
    print(f"{len(TEXTS)} vectors written to {out_path}")


if __name__ == "__main__":
    main()
