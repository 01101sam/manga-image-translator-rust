## Usage

```sh
❯ cargo r -p simple-runtime -- cli -i path/to/input -o path/to/output
❯ ./runtime cli -i path/to/input -o path/to/output

Usage: simple-runtime cli [OPTIONS] --input <INPUT> --output <OUTPUT>

Options:
  -i, --input <INPUT>
          Input file or directory
  -v, --verbose...
          Verbose mode (-v, -vv, -vvv)
      --max-batch-size-ocr <MAX_BATCH_SIZE_OCR>
          maximum batch size for ocr [default: 16]
  -o, --output <OUTPUT>
          Output directory
  -c, --config <CONFIG>
          Optional config file
      --max-batch-size-upscaler <MAX_BATCH_SIZE_UPSCALER>
          maximum batch size for upscaler [default: 2]
      --overwrite
          Overwrite already translated images
  -h, --help
          Print help


Usage: simple-runtime [OPTIONS] <COMMAND>

Commands:
  cli   Run the image translation CLI
  api   Run in API server mode
  help  Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose...
          Verbose mode (-v, -vv, -vvv)
      --max-batch-size-ocr <MAX_BATCH_SIZE_OCR>
          maximum batch size for ocr [default: 16]
      --max-batch-size-upscaler <MAX_BATCH_SIZE_UPSCALER>
          maximum batch size for upscaler [default: 2]
  -h, --help
          Print help
  -V, --version
          Print version
```

Only
- coreml
- cuda
- cpu
- tensorrt
- (rocm) needs compile from source with --features "rocm"

is supported right now. For AMD support look at how to enable rocm for onnxruntime or maybe ZLUDA
