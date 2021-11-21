use std::io::{Seek, SeekFrom, Write};

use binwrite::BinWrite;

use crate::{image::ImageAssemblyEntry, Image, WanError};

pub enum CompressionMethod {
    CompressionMethodOriginal,
    CompressionMethodOptimised {
        multiple_of_value: usize,
        min_transparent_to_compress: usize,
    },
    NoCompression,
}

impl CompressionMethod {
    pub fn compress<F: Write + Seek>(
        &self,
        image: &Image,
        pixel_list: &[u8],
        file: &mut F,
    ) -> Result<Vec<ImageAssemblyEntry>, WanError> {
        let mut assembly_table: Vec<ImageAssemblyEntry> = vec![];
        match self {
            Self::CompressionMethodOriginal => {
                enum ActualEntry {
                    Null(u64, u32),      //lenght (pixel), z_index
                    Some(u64, u64, u32), // initial_offset, lenght (pixel), z_index
                }

                impl ActualEntry {
                    fn new(is_all_black: bool, start_offset: u64, z_index: u32) -> ActualEntry {
                        if is_all_black {
                            ActualEntry::Null(64, z_index)
                        } else {
                            ActualEntry::Some(start_offset, 64, z_index)
                        }
                    }

                    fn to_assembly(&self) -> ImageAssemblyEntry {
                        match self {
                            ActualEntry::Null(lenght, z_index) => ImageAssemblyEntry {
                                pixel_src: 0,
                                pixel_amount: *lenght,
                                byte_amount: *lenght / 2,
                                _z_index: *z_index,
                            },
                            ActualEntry::Some(initial_offset, lenght, z_index) => {
                                ImageAssemblyEntry {
                                    pixel_src: *initial_offset,
                                    pixel_amount: *lenght,
                                    byte_amount: *lenght / 2,
                                    _z_index: *z_index,
                                }
                            }
                        }
                    }

                    fn advance(&self, lenght: u64) -> ActualEntry {
                        match self {
                            ActualEntry::Null(l, z) => ActualEntry::Null(*l + lenght, *z),
                            ActualEntry::Some(offset, l, z) => {
                                ActualEntry::Some(*offset, *l + lenght, *z)
                            }
                        }
                    }
                }

                let mut actual_entry: Option<ActualEntry> = None;

                for loop_nb in 0..(image.img.width() / 8 * image.img.height() / 8) {
                    let mut this_area = vec![];
                    let mut is_all_black = true;
                    for l in 0..64 {
                        let actual_pixel = pixel_list[(loop_nb * 64 + l) as usize];
                        this_area.push(actual_pixel);
                        if actual_pixel != 0 {
                            is_all_black = false;
                        };
                    }

                    let pos_before_area = file.seek(SeekFrom::Current(0))?;
                    if !is_all_black {
                        for byte_id in 0..32 {
                            (((this_area[byte_id * 2] << 4) + this_area[byte_id * 2 + 1]) as u8)
                                .write(file)?;
                        }
                    }

                    let need_to_create_new_entry = if actual_entry.is_none() {
                        true
                    } else {
                        match &actual_entry {
                            Some(ActualEntry::Null(_, _)) => !is_all_black,
                            Some(ActualEntry::Some(_, _, _)) => is_all_black,
                            _ => panic!(),
                        }
                    };

                    if need_to_create_new_entry {
                        if let Some(entry) = actual_entry {
                            assembly_table.push(entry.to_assembly())
                        }

                        actual_entry = Some(ActualEntry::new(
                            is_all_black,
                            pos_before_area,
                            image.z_index,
                        ));
                    } else {
                        //TODO:
                        actual_entry = Some(actual_entry.unwrap().advance(64));
                    }
                }
                assembly_table.push(actual_entry.unwrap().to_assembly())
            }
            Self::CompressionMethodOptimised {
                multiple_of_value,
                min_transparent_to_compress,
            } => {
                let mut number_of_byte_to_include = 0;
                let mut byte_include_start = file.seek(SeekFrom::Current(0))?;

                let mut pixel_id = 0;
                loop {
                    debug_assert!(pixel_id % 2 == 0);
                    let mut should_create_new_transparent_entry = false;

                    if (pixel_id % multiple_of_value == 0)
                        && (pixel_id + min_transparent_to_compress < pixel_list.len())
                    {
                        let mut encontered_non_transparent = false;
                        for l in 0..*min_transparent_to_compress {
                            if pixel_list[pixel_id + l] != 0 {
                                encontered_non_transparent = true;
                                break;
                            };
                        }
                        if !encontered_non_transparent {
                            should_create_new_transparent_entry = true;
                        };
                    };

                    if should_create_new_transparent_entry {
                        //push the actual content
                        if number_of_byte_to_include > 0 {
                            assembly_table.push(ImageAssemblyEntry {
                                pixel_src: byte_include_start,
                                pixel_amount: number_of_byte_to_include * 2,
                                byte_amount: number_of_byte_to_include,
                                _z_index: image.z_index,
                            });
                            number_of_byte_to_include = 0;
                            byte_include_start = file.seek(SeekFrom::Current(0))?;
                        };
                        //create new entry for transparent stuff
                        //count the number of transparent tile
                        let mut transparent_tile_nb = 0;
                        loop {
                            if pixel_id >= pixel_list.len() {
                                break;
                            };
                            if pixel_list[pixel_id] == 0 {
                                transparent_tile_nb += 1;
                                pixel_id += 1;
                            } else {
                                break;
                            };
                        }
                        if pixel_id % multiple_of_value != 0 {
                            transparent_tile_nb -= pixel_id % multiple_of_value;
                            pixel_id -= pixel_id % multiple_of_value;
                        };
                        assembly_table.push(ImageAssemblyEntry {
                            pixel_src: 0,
                            pixel_amount: transparent_tile_nb as u64,
                            byte_amount: (transparent_tile_nb as u64) / 2, //TODO: take care of the tileset lenght
                            _z_index: image.z_index,
                        });

                        continue;
                    };

                    if pixel_id >= pixel_list.len() {
                        break;
                    };
                    debug_assert!(pixel_list[pixel_id] < 16);
                    debug_assert!(pixel_list[pixel_id + 1] < 16);
                    (((pixel_list[pixel_id] << 4) + pixel_list[pixel_id + 1]) as u8).write(file)?;
                    pixel_id += 2;
                    number_of_byte_to_include += 1;
                }
                if number_of_byte_to_include > 0 {
                    assembly_table.push(ImageAssemblyEntry {
                        pixel_src: byte_include_start,
                        pixel_amount: number_of_byte_to_include * 2,
                        byte_amount: number_of_byte_to_include,
                        _z_index: image.z_index,
                    });
                };
            }
            Self::NoCompression => {
                let mut byte_len = 0;
                let start_offset = file.seek(SeekFrom::Current(0))?;
                for pixels in pixel_list.chunks_exact(2) {
                    ((pixels[0] << 4) + pixels[1]).write(file)?;
                    byte_len += 1;
                }
                assembly_table.push(ImageAssemblyEntry {
                    pixel_src: start_offset,
                    pixel_amount: byte_len * 2,
                    byte_amount: byte_len,
                    _z_index: image.z_index,
                })
            }
        };
        Ok(assembly_table)
    }
}
