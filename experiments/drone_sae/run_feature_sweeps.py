#!/usr/bin/env python3

import argparse
import json
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import torch
from diffusers import AutoPipelineForImage2Image
from PIL import Image, ImageDraw, ImageFont
from torch import nn


MODEL_ID = "stabilityai/sdxl-turbo"
PROMPT = (
    "A photorealistic overhead drone survey photograph of a land restoration plot, "
    "natural vegetation, soil and paths, preserving the exact terrain layout"
)
BLOCKS = {
    "composition": "unet.down_blocks.2.attentions.1",
    "detail": "unet.up_blocks.0.attentions.0",
    "style": "unet.up_blocks.0.attentions.1",
}
TIMES = ("1000", "1200", "1500")


class SparseAutoencoder(nn.Module):
    def __init__(self, feature_count: int, model_width: int, top_k: int) -> None:
        super().__init__()
        self.top_k = top_k
        self.encoder = nn.Linear(model_width, feature_count, bias=False)
        self.decoder = nn.Linear(feature_count, model_width, bias=False)
        self.pre_bias = nn.Parameter(torch.zeros(model_width))
        self.latent_bias = nn.Parameter(torch.zeros(feature_count))
        self.register_buffer(
            "stats_last_nonzero", torch.zeros(feature_count, dtype=torch.long)
        )

    def encode(self, activations: torch.Tensor) -> torch.Tensor:
        pre_activations = self.encoder(activations - self.pre_bias) + self.latent_bias
        values, indices = torch.topk(pre_activations, k=self.top_k, dim=-1)
        sparse = torch.zeros_like(pre_activations)
        sparse.scatter_(-1, indices, torch.relu(values))
        return sparse


@dataclass(frozen=True)
class LoadedSae:
    model: SparseAutoencoder
    mean_activation: torch.Tensor


@dataclass(frozen=True)
class InputImage:
    label: str
    plot: str
    time: str
    path: Path
    image: Image.Image


@dataclass(frozen=True)
class ScreenedFeature:
    block: str
    index: int
    effect: float
    chroma_effect: float
    signature: np.ndarray


@dataclass(frozen=True)
class SelectedFeature:
    source: str
    block: str
    index: int
    screen_effect: float
    screen_chroma_effect: float


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Build a strong steering atlas from discovered and random active "
            "pretrained SDXL sparse-autoencoder features."
        )
    )
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--plots", nargs="+", required=True)
    parser.add_argument("--checkpoint-root", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--steps", type=int, default=4)
    parser.add_argument("--image-strength", type=float, default=0.25)
    parser.add_argument(
        "--feature-strengths",
        type=float,
        nargs="+",
        default=[-120.0, -80.0, -40.0, 0.0, 40.0, 80.0, 120.0],
    )
    parser.add_argument("--screen-strength", type=float, default=70.0)
    parser.add_argument("--screen-candidates", type=int, default=32)
    parser.add_argument("--discovered-per-block", type=int, default=3)
    parser.add_argument("--color-per-block", type=int, default=1)
    parser.add_argument("--random-per-block", type=int, default=2)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--cell-size", type=int, default=224)
    return parser.parse_args()


def locate_module(root: object, dotted_path: str) -> nn.Module:
    current = root
    for component in dotted_path.split("."):
        current = (
            current[int(component)]
            if component.isdigit()
            else getattr(current, component)
        )
    if not isinstance(current, nn.Module):
        raise TypeError(f"{dotted_path} did not resolve to a torch module")
    return current


def checkpoint_dir(root: Path, module_path: str) -> Path:
    matches = sorted(root.glob(f"{module_path}_k10_hidden5120_*/final"))
    if len(matches) != 1:
        raise RuntimeError(
            f"Expected one checkpoint for {module_path}, found {len(matches)}"
        )
    return matches[0]


def load_sae(root: Path, module_path: str, device: str) -> LoadedSae:
    directory = checkpoint_dir(root, module_path)
    config = json.loads((directory / "config.json").read_text())
    sae = SparseAutoencoder(
        feature_count=config["n_dirs_local"],
        model_width=config["d_model"],
        top_k=config["k"],
    )
    payload = torch.load(
        directory / "state_dict.pth", map_location="cpu", weights_only=True
    )
    state = payload["state_dict"]
    state.pop("auxk_mask_fn", None)
    sae.load_state_dict(state)
    sae.to(device=device, dtype=torch.float16).eval()
    means = torch.load(
        directory / "mean.pt", map_location=device, weights_only=True
    ).to(dtype=torch.float16)
    return LoadedSae(model=sae, mean_activation=means)


def load_images(input_dir: Path, plots: list[str]) -> list[InputImage]:
    loaded = []
    for plot in plots:
        for time_name in TIMES:
            path = input_dir / f"{plot}__time_{time_name}.jpg"
            if not path.is_file():
                raise FileNotFoundError(path)
            image = (
                Image.open(path)
                .convert("RGB")
                .resize((512, 512), Image.Resampling.LANCZOS)
            )
            loaded.append(
                InputImage(
                    label=f"{plot} / {time_name}",
                    plot=plot,
                    time=time_name,
                    path=path,
                    image=image,
                )
            )
    return loaded


def generator(seed: int) -> torch.Generator:
    return torch.Generator(device="cpu").manual_seed(seed)


@torch.inference_mode()
def discover_features(
    pipeline: AutoPipelineForImage2Image,
    saes: dict[str, LoadedSae],
    image: Image.Image,
    steps: int,
    image_strength: float,
    seed: int,
) -> tuple[Image.Image, dict[str, torch.Tensor]]:
    captured: dict[str, list[torch.Tensor]] = {name: [] for name in BLOCKS}
    handles = []

    for block_name, module_path in BLOCKS.items():
        module = locate_module(pipeline, module_path)
        sae = saes[block_name].model

        def capture_hook(
            _module: nn.Module,
            inputs: tuple[torch.Tensor, ...],
            output: tuple[torch.Tensor, ...],
            *,
            feature_name: str = block_name,
            feature_sae: SparseAutoencoder = sae,
        ) -> None:
            update = (output[0] - inputs[0]).permute(0, 2, 3, 1)
            sparse = feature_sae.encode(update)
            captured[feature_name].append(
                sparse.float().mean(dim=(0, 1, 2)).cpu()
            )

        handles.append(module.register_forward_hook(capture_hook))

    try:
        result = pipeline(
            prompt=PROMPT,
            image=image,
            strength=image_strength,
            num_inference_steps=steps,
            guidance_scale=0.0,
            generator=generator(seed),
        ).images[0]
    finally:
        for handle in handles:
            handle.remove()

    scores = {
        name: torch.stack(per_step).mean(dim=0)
        for name, per_step in captured.items()
    }
    return result, scores


def aggregate_feature_scores(
    scores_by_image: dict[str, dict[str, torch.Tensor]]
) -> dict[str, dict[str, torch.Tensor]]:
    aggregates = {}
    for block_name in BLOCKS:
        matrix = torch.stack(
            [per_block[block_name] for per_block in scores_by_image.values()]
        )
        prevalence = (matrix > 0).float().mean(dim=0)
        mean_score = matrix.mean(dim=0)
        aggregates[block_name] = {
            "mean": mean_score,
            "prevalence": prevalence,
            "ranking": mean_score * prevalence.sqrt(),
        }
    return aggregates


def replace_first_output(
    output: torch.Tensor | tuple[torch.Tensor, ...], edited: torch.Tensor
) -> torch.Tensor | tuple[torch.Tensor, ...]:
    if isinstance(output, tuple):
        return (edited, *output[1:])
    return edited


@torch.inference_mode()
def apply_feature(
    pipeline: AutoPipelineForImage2Image,
    loaded_sae: LoadedSae,
    module_path: str,
    feature_index: int,
    feature_strength: float,
    image: Image.Image,
    steps: int,
    image_strength: float,
    seed: int,
) -> Image.Image:
    module = locate_module(pipeline, module_path)
    decoder_direction = loaded_sae.model.decoder.weight[:, feature_index]
    activation_scale = loaded_sae.mean_activation[feature_index] * feature_strength

    def steering_hook(
        _module: nn.Module,
        _inputs: tuple[torch.Tensor, ...],
        output: torch.Tensor | tuple[torch.Tensor, ...],
    ) -> torch.Tensor | tuple[torch.Tensor, ...]:
        hidden = output[0] if isinstance(output, tuple) else output
        delta = (
            decoder_direction.to(hidden)
            .view(1, -1, 1, 1)
            .mul(activation_scale.to(hidden))
        )
        return replace_first_output(output, hidden + delta)

    handle = module.register_forward_hook(steering_hook)
    try:
        return pipeline(
            prompt=PROMPT,
            image=image,
            strength=image_strength,
            num_inference_steps=steps,
            guidance_scale=0.0,
            generator=generator(seed),
        ).images[0]
    finally:
        handle.remove()


def transplant_pixel_delta(
    source: Image.Image, baseline: Image.Image, steered: Image.Image
) -> Image.Image:
    source_pixels = np.asarray(source, dtype=np.float32)
    baseline_pixels = np.asarray(baseline, dtype=np.float32)
    steered_pixels = np.asarray(steered, dtype=np.float32)
    edited_pixels = np.clip(
        np.rint(source_pixels + steered_pixels - baseline_pixels), 0, 255
    ).astype(np.uint8)
    return Image.fromarray(edited_pixels)


def effect_and_signature(
    baseline: Image.Image, steered: Image.Image
) -> tuple[float, float, np.ndarray]:
    delta = (
        np.asarray(steered, dtype=np.float32)
        - np.asarray(baseline, dtype=np.float32)
    )
    effect = float(np.abs(delta).mean())
    chroma_delta = delta - delta.mean(axis=2, keepdims=True)
    chroma_effect = float(np.abs(chroma_delta).mean())
    pooled = delta.reshape(32, 16, 32, 16, 3).mean(axis=(1, 3)).reshape(-1)
    norm = float(np.linalg.norm(pooled))
    signature = pooled / max(norm, 1e-8)
    return effect, chroma_effect, signature


def screen_candidates(
    pipeline: AutoPipelineForImage2Image,
    saes: dict[str, LoadedSae],
    aggregates: dict[str, dict[str, torch.Tensor]],
    representative: InputImage,
    representative_baseline: Image.Image,
    candidate_count: int,
    strength: float,
    steps: int,
    image_strength: float,
    seed: int,
) -> dict[str, list[ScreenedFeature]]:
    screened = {}
    for block_name in BLOCKS:
        candidate_indices = torch.topk(
            aggregates[block_name]["ranking"], k=candidate_count
        ).indices.tolist()
        block_features = []
        for feature_index in candidate_indices:
            steered = apply_feature(
                pipeline,
                saes[block_name],
                BLOCKS[block_name],
                feature_index,
                strength,
                representative.image,
                steps,
                image_strength,
                seed,
            )
            effect, chroma_effect, signature = effect_and_signature(
                representative_baseline, steered
            )
            block_features.append(
                ScreenedFeature(
                    block=block_name,
                    index=feature_index,
                    effect=effect,
                    chroma_effect=chroma_effect,
                    signature=signature,
                )
            )
        screened[block_name] = block_features
    return screened


def select_diverse_discovered(
    screened: dict[str, list[ScreenedFeature]], count: int
) -> list[SelectedFeature]:
    selected = []
    for block_name, candidates in screened.items():
        remaining = list(candidates)
        chosen: list[ScreenedFeature] = []
        while len(chosen) < count:
            def utility(candidate: ScreenedFeature) -> float:
                if not chosen:
                    return candidate.effect
                similarity = max(
                    abs(float(np.dot(candidate.signature, prior.signature)))
                    for prior in chosen
                )
                return candidate.effect * (0.2 + 0.8 * (1.0 - similarity))

            best = max(remaining, key=utility)
            remaining.remove(best)
            chosen.append(best)
        selected.extend(
            SelectedFeature(
                source="discovered",
                block=item.block,
                index=item.index,
                screen_effect=item.effect,
                screen_chroma_effect=item.chroma_effect,
            )
            for item in chosen
        )
    return selected


def select_color_discovered(
    screened: dict[str, list[ScreenedFeature]],
    excluded: set[tuple[str, int]],
    count: int,
) -> list[SelectedFeature]:
    selected = []
    for block_name, candidates in screened.items():
        available = [
            candidate
            for candidate in candidates
            if (block_name, candidate.index) not in excluded
        ]
        available.sort(key=lambda candidate: candidate.chroma_effect, reverse=True)
        selected.extend(
            SelectedFeature(
                source="color-discovered",
                block=candidate.block,
                index=candidate.index,
                screen_effect=candidate.effect,
                screen_chroma_effect=candidate.chroma_effect,
            )
            for candidate in available[:count]
        )
    return selected


def select_random_active(
    aggregates: dict[str, dict[str, torch.Tensor]],
    excluded: set[tuple[str, int]],
    count: int,
    seed: int,
) -> list[SelectedFeature]:
    rng = np.random.default_rng(seed)
    selected = []
    for block_name in BLOCKS:
        prevalence = aggregates[block_name]["prevalence"]
        active = [
            int(index)
            for index in torch.nonzero(prevalence > 0, as_tuple=False).flatten()
            if (block_name, int(index)) not in excluded
        ]
        indices = rng.choice(active, size=count, replace=False)
        selected.extend(
            SelectedFeature(
                source="random-active",
                block=block_name,
                index=int(index),
                screen_effect=float("nan"),
                screen_chroma_effect=float("nan"),
            )
            for index in indices
        )
    return selected


def measure_selected_random_features(
    pipeline: AutoPipelineForImage2Image,
    saes: dict[str, LoadedSae],
    selected: list[SelectedFeature],
    representative: InputImage,
    representative_baseline: Image.Image,
    strength: float,
    steps: int,
    image_strength: float,
    seed: int,
) -> list[SelectedFeature]:
    measured = []
    for feature in selected:
        if feature.source != "random-active":
            measured.append(feature)
            continue
        steered = apply_feature(
            pipeline,
            saes[feature.block],
            BLOCKS[feature.block],
            feature.index,
            strength,
            representative.image,
            steps,
            image_strength,
            seed,
        )
        effect, chroma_effect, _signature = effect_and_signature(
            representative_baseline, steered
        )
        measured.append(
            SelectedFeature(
                source=feature.source,
                block=feature.block,
                index=feature.index,
                screen_effect=effect,
                screen_chroma_effect=chroma_effect,
            )
        )
    return measured


def make_grid(
    source_rows: list[InputImage],
    rendered: dict[str, dict[float, Image.Image]],
    strengths: list[float],
    title: str,
    output_path: Path,
    cell_size: int,
) -> None:
    font = ImageFont.load_default(size=max(14, cell_size // 16))
    label_width = max(240, cell_size)
    header_height = max(90, cell_size // 3)
    columns = ["source", *[f"{value:+g}" for value in strengths]]
    canvas = Image.new(
        "RGB",
        (
            label_width + len(columns) * cell_size,
            header_height + len(source_rows) * cell_size,
        ),
        "white",
    )
    draw = ImageDraw.Draw(canvas)
    draw.text((12, 10), title, fill="black", font=font)
    for column_index, label in enumerate(columns):
        draw.text(
            (label_width + column_index * cell_size + 12, header_height // 2),
            label,
            fill="black",
            font=font,
        )
    for row_index, source in enumerate(source_rows):
        y = header_height + row_index * cell_size
        draw.text((12, y + 12), source.label, fill="black", font=font)
        canvas.paste(
            source.image.resize((cell_size, cell_size)),
            (label_width, y),
        )
        for strength_index, strength in enumerate(strengths):
            edited = rendered[source.label][strength].resize(
                (cell_size, cell_size)
            )
            canvas.paste(
                edited,
                (label_width + (strength_index + 1) * cell_size, y),
            )
    canvas.save(output_path, quality=92, optimize=True)


def main() -> None:
    args = parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    images = load_images(args.input_dir, args.plots)

    pipeline = AutoPipelineForImage2Image.from_pretrained(
        MODEL_ID,
        torch_dtype=torch.float16,
        variant="fp16",
        use_safetensors=True,
    ).to("cuda")
    pipeline.set_progress_bar_config(disable=True)
    pipeline.enable_vae_slicing()

    saes = {
        name: load_sae(args.checkpoint_root, module_path, "cuda")
        for name, module_path in BLOCKS.items()
    }

    baseline_by_image = {}
    scores_by_image = {}
    for input_image in images:
        baseline, scores = discover_features(
            pipeline,
            saes,
            input_image.image,
            args.steps,
            args.image_strength,
            args.seed,
        )
        baseline_by_image[input_image.label] = baseline
        scores_by_image[input_image.label] = scores

    aggregates = aggregate_feature_scores(scores_by_image)
    representative = images[len(images) // 2 + 1]
    screened = screen_candidates(
        pipeline,
        saes,
        aggregates,
        representative,
        baseline_by_image[representative.label],
        args.screen_candidates,
        args.screen_strength,
        args.steps,
        args.image_strength,
        args.seed,
    )
    selected = select_diverse_discovered(
        screened, args.discovered_per_block
    )
    excluded = {(feature.block, feature.index) for feature in selected}
    selected.extend(
        select_color_discovered(
            screened,
            excluded,
            args.color_per_block,
        )
    )
    excluded = {(feature.block, feature.index) for feature in selected}
    selected.extend(
        select_random_active(
            aggregates,
            excluded,
            args.random_per_block,
            args.seed,
        )
    )
    selected = measure_selected_random_features(
        pipeline,
        saes,
        selected,
        representative,
        baseline_by_image[representative.label],
        args.screen_strength,
        args.steps,
        args.image_strength,
        args.seed,
    )

    manifest_features = []
    for sequence, feature in enumerate(selected, start=1):
        rendered = {}
        for input_image in images:
            rendered[input_image.label] = {}
            for strength in args.feature_strengths:
                if strength == 0:
                    edited = input_image.image
                else:
                    steered = apply_feature(
                        pipeline,
                        saes[feature.block],
                        BLOCKS[feature.block],
                        feature.index,
                        strength,
                        input_image.image,
                        args.steps,
                        args.image_strength,
                        args.seed,
                    )
                    edited = transplant_pixel_delta(
                        input_image.image,
                        baseline_by_image[input_image.label],
                        steered,
                    )
                rendered[input_image.label][strength] = edited

        grid_path = args.output_dir / (
            f"{sequence:02d}_{feature.source}_{feature.block}_{feature.index}.jpg"
        )
        make_grid(
            images,
            rendered,
            args.feature_strengths,
            (
                f"{feature.source} | {feature.block} feature {feature.index} | "
                f"effect {feature.screen_effect:.2f} px | "
                f"chroma {feature.screen_chroma_effect:.2f} px"
            ),
            grid_path,
            args.cell_size,
        )
        block_aggregate = aggregates[feature.block]
        manifest_features.append(
            {
                "sequence": sequence,
                "source": feature.source,
                "block": feature.block,
                "module": BLOCKS[feature.block],
                "feature_index": feature.index,
                "screen_effect_mean_absolute_pixels": feature.screen_effect,
                "screen_chroma_effect_mean_absolute_pixels": (
                    feature.screen_chroma_effect
                ),
                "mean_activation_on_inputs": float(
                    block_aggregate["mean"][feature.index].item()
                ),
                "activation_prevalence_on_inputs": float(
                    block_aggregate["prevalence"][feature.index].item()
                ),
                "checkpoint_mean_activation": float(
                    saes[feature.block]
                    .mean_activation[feature.index]
                    .float()
                    .item()
                ),
                "grid": grid_path.name,
            }
        )
        print(
            f"[{sequence}/{len(selected)}] wrote {grid_path.name}",
            flush=True,
        )

    manifest = {
        "model": MODEL_ID,
        "prompt": PROMPT,
        "plots": args.plots,
        "inputs": [image.path.name for image in images],
        "steps": args.steps,
        "image_strength": args.image_strength,
        "feature_strengths": args.feature_strengths,
        "screen_strength": args.screen_strength,
        "screen_candidates_per_block": args.screen_candidates,
        "discovered_per_block": args.discovered_per_block,
        "color_discovered_per_block": args.color_per_block,
        "random_active_per_block": args.random_per_block,
        "seed": args.seed,
        "selection": (
            "Discovered features are greedily selected for causal pixel effect "
            "and effect-pattern diversity from the highest activation-ranked "
            "candidates. Color-discovered features separately maximize causal "
            "RGB-channel chroma change. Random features are a seeded uniform "
            "sample from dictionary entries active on at least one input."
        ),
        "pixel_application": (
            "source pixels + (steered diffusion output - baseline diffusion output)"
        ),
        "features": manifest_features,
    }
    (args.output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
