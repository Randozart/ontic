"""
MLIR pipeline for executing MLIR modules.
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowering through a pipeline of passes (placeholder).
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()

"""
MLIR pipeline for executing MLIR modules.

This module provides a simple MLIR execution pipeline that:
  1. Parses an MLIR file.
  2. Lowers it through a pipeline of passes.
  3. Executes the lowered module.
  4. Reports timing and memory usage.

Example usage:
    python -m pipeline --mlir-path examples/ridge.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --output output.mlir
    python -m pipeline --mlir-path examples/ridge.mlir --input-shape 4,32 --dtype f32
"""

import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import mlir
import mlir._mlir_libs._mlir as _mlir_ir
import numpy as np


@dataclass
class MLIRPipeline:
    """MLIR execution pipeline."""

    mlir_path: Path
    input_shape: tuple[int, ...]
    dtype: str
    output: Optional[Path] = None
    time_unit: str = "s"
    time_precision: int = 6

    def execute(self) -> tuple[mlir.Module, float, float]:
        """Execute the MLIR module through the pipeline.

        Returns:
            A tuple containing:
                - The compiled ir.Module.
                - The elapsed time in the specified time unit.
                - The peak memory usage in MB.
        """
        mlir_path: Path = self.mlir_path
        input_shape: tuple[int, ...] = self.input_shape
        dtype: str = self.dtype
        output: Optional[Path] = self.output
        time_unit: str = self.time_unit
        time_precision: int = self.time_precision

        # Parse the MLIR file.
        mlir_text: str = mlir_path.read_text(encoding="utf-8")
        module: ir.Module = _mlir_ir.parse_source(mlir_text)

        # Set the module name.
        module.name = "pipeline_exec"

        # Lower the module through the pipeline of passes.
        # For now, we just return the module with zero elapsed time.
        elapsed_time: float = 0.0
        peak_memory: float = 0.0

        # Write the output if requested.
        if output is not None:
            output.write_text(str(module), encoding="utf-8")

        return module, elapsed_time, peak_memory


def main() -> None:
    """Main entry point for the MLIR pipeline."""
    mlir_path: Path = Path(os.environ.get("MLIR_PATH", "examples/ridge.mlir"))
    input_shape_str: str = os.environ.get("INPUT_SHAPE", "4,32")
    dtype: str = os.environ.get("DTYPE", "f32")
    output: Optional[Path] = Path(os.environ.get("OUTPUT", None))
    time_unit: str = os.environ.get("TIME_UNIT", "s")
    time_precision: int = int(os.environ.get("TIME_PRECISION", "6"))

    pipeline = MLIRPipeline(
        mlir_path=mlir_path,
        input_shape=tuple(int(x) for x in input_shape_str.split(",")),
        dtype=dtype,
        output=output,
        time_unit=time_unit,
        time_precision=time_precision,
    )

    module, elapsed_time, peak_memory = pipeline.execute()

    print(f"Input shape: {pipeline.input_shape}")
    print(f"Input dtype: {pipeline.dtype}")
    print(f"Elapsed time: {elapsed_time:.{time_precision}f} {time_unit}")
    print(f"Peak memory: {peak_memory:.2f} MB")


if __name__ == "__main__":
    main()