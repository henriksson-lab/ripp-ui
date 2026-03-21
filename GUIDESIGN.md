# Overall GUI design principle

# Local use vs Server

This software is designed to be able to run both locally on your own computer, and remotely in a compute cluster ("server mode").

# Tools and work area

The left side of the software is for all tools while the right side is where an image is shown (from microscope or data).

micromanager-rs-llm will be used to interface microscopy hardware.


# Caching of data

Because the image data is typically huge, this software aims to only cache the needed data in memory.
Data is stored in S3 (ome-zarr), data can also be retrived using bioformats-rs-llm.
