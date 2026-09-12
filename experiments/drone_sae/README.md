# Drone image SAE feature sweeps

This experiment applies the pretrained sparse-autoencoder dictionaries from
[SDXL-Unbox](https://github.com/surkovv/sdxl-unbox) to real drone-image trios.
It does not train or fine-tune SDXL or an SAE.

For each of SDXL-Turbo's composition, detail, and style blocks, the script:

1. Runs image-to-image inference on every requested plot/time image.
2. Encodes each block's residual update with its released TopK SAE.
3. Causally screens high-activation candidates and selects features with strong,
   diverse pixel effects.
4. Separately selects features with strong causal chroma effects.
5. Separately samples reproducibly random dictionary entries that activated on
   at least one input.
6. Adds each feature's decoder direction at strong negative and positive
   strengths.
7. Isolates the causal output delta and adds it to the untouched source pixels:
   `source + (steered output - baseline output)`.
8. Produces one multi-image grid per feature and a JSON manifest.

The upstream checkpoints are expected in the layout produced by extracting the
SDXL-Unbox repository archive.

```bash
python run_feature_sweeps.py \
  --input-dir /path/to/drone-lsr-images-rest \
  --plots plot_10_10 plot_15_20 plot_20_30 plot_5_5 \
  --checkpoint-root /path/to/sdxl-unbox/checkpoints \
  --output-dir outputs/atlas_4plots_strong
```

The defaults use four SDXL-Turbo steps, 25% image-to-image noise, and feature
strengths `-120 -80 -40 0 40 80 120`. Three causally strong/diverse and two
random-active features are selected from each block, plus one color-focused
feature from each block, for 18 feature grids.
