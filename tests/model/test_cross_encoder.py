import torch

from src.model.cross_encoder import CrossEncoderReranker


def test_forward_returns_scores():
    model = CrossEncoderReranker("microsoft/deberta-v3-xsmall")
    tokenizer = model.tokenizer

    queries = ["what is python", "rust async"]
    documents = ["Python is a programming language", "Rust uses async/await for concurrency"]

    encodings = tokenizer(
        queries,
        documents,
        padding=True,
        truncation=True,
        max_length=128,
        return_tensors="pt",
    )

    scores = model(
        input_ids=encodings["input_ids"],
        attention_mask=encodings["attention_mask"],
        token_type_ids=encodings.get("token_type_ids"),
    )

    assert scores.shape == (2,)
    assert all(0.0 <= s <= 1.0 for s in scores.tolist())


def test_forward_with_labels_returns_loss():
    model = CrossEncoderReranker("microsoft/deberta-v3-xsmall")
    tokenizer = model.tokenizer

    queries = ["what is python"]
    documents = ["Python is a programming language"]
    labels = torch.tensor([1.0])

    encodings = tokenizer(
        queries,
        documents,
        padding=True,
        truncation=True,
        max_length=128,
        return_tensors="pt",
    )

    loss, scores = model(
        input_ids=encodings["input_ids"],
        attention_mask=encodings["attention_mask"],
        token_type_ids=encodings.get("token_type_ids"),
        labels=labels,
    )

    assert loss.dim() == 0  # scalar
    assert loss.item() > 0
    assert scores.shape == (1,)


def test_save_and_load(tmp_path):
    model = CrossEncoderReranker("microsoft/deberta-v3-xsmall")
    model.save(str(tmp_path / "test_model"))

    loaded = CrossEncoderReranker.load(str(tmp_path / "test_model"))

    assert loaded.tokenizer is not None
    assert loaded.backbone.config.model_type == model.backbone.config.model_type
