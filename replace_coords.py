import re

with open('src/table_function.rs', 'r') as f:
    content = f.read()

# First part: macro dispatch_yield_loop
old_macro = """            for i in 0..batch_size {
                let local_idx = $state.local_chunk_cursor + i;
                let global_coords = crate::table_function::calculate_global_indices(
                    local_idx,
                    &$bind_data.chunk_shape,
                    &$state.current_chunk_grid,
                );

                // Ghost row check: Ensure global coords do not exceed array shape bounds
                let mut out_of_bounds = false;
                for dim in 0..rank {
                    if global_coords[dim] > $bind_data.bounds_max[dim] {
                        out_of_bounds = true;
                        break;
                    }
                }
                if out_of_bounds {
                    // Advance cursor but do not write output
                    continue;
                }

                // Write coordinates
                for dim in 0..rank {
                    if $state.projected_columns.contains(&dim) {
                        if let Some(coord_vals) = $bind_data.coords.get(&$bind_data.dim_names[dim])
                        {
                            let mut coord_vector = $output.flat_vector(dim);
                            let coord_slice = coord_vector.as_mut_slice::<f64>();
                            coord_slice[valid_rows] = coord_vals
                                .get(global_coords[dim] as usize)
                                .copied()
                                .unwrap_or(f64::NAN);
                        } else {
                            let mut coord_vector = $output.flat_vector(dim);
                            let coord_slice = coord_vector.as_mut_slice::<i64>();
                            coord_slice[valid_rows] = global_coords[dim] as i64;
                        }
                    }
                }

                // Write value
                if $state.projected_columns.contains(&rank) {
                    let val = buffer[local_idx];
                    let val_bytes = val.to_ne_bytes();
                    let is_fill = val_bytes.as_ref() == fill_bytes_slice;

                    if is_fill {
                        // Set NULL
                        value_vector.set_null(valid_rows);
                    } else {
                        value_vector.as_mut_slice::<$rust_type>()[valid_rows] = val;
                    }
                }

                valid_rows += 1;
            }"""

new_macro = """            let mut valid_coords = Vec::with_capacity(batch_size);
            for i in 0..batch_size {
                let local_idx = $state.local_chunk_cursor + i;
                let global_coords = crate::table_function::calculate_global_indices(
                    local_idx,
                    &$bind_data.chunk_shape,
                    &$state.current_chunk_grid,
                );

                // Ghost row check: Ensure global coords do not exceed array shape bounds
                let mut out_of_bounds = false;
                for dim in 0..rank {
                    if global_coords[dim] > $bind_data.bounds_max[dim] {
                        out_of_bounds = true;
                        break;
                    }
                }
                if !out_of_bounds {
                    valid_coords.push((local_idx, global_coords));
                }
            }

            // Write coordinates
            for dim in 0..rank {
                if $state.projected_columns.contains(&dim) {
                    if let Some(coord_vals) = $bind_data.coords.get(&$bind_data.dim_names[dim]) {
                        let mut coord_vector = $output.flat_vector(dim);
                        let coord_slice = coord_vector.as_mut_slice::<f64>();
                        for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                            coord_slice[valid_rows + idx] = coord_vals
                                .get(global_coords[dim] as usize)
                                .copied()
                                .unwrap_or(f64::NAN);
                        }
                    } else {
                        let mut coord_vector = $output.flat_vector(dim);
                        let coord_slice = coord_vector.as_mut_slice::<i64>();
                        for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                            coord_slice[valid_rows + idx] = global_coords[dim] as i64;
                        }
                    }
                }
            }

            // Write value
            if $state.projected_columns.contains(&rank) {
                for (idx, (local_idx, _)) in valid_coords.iter().enumerate() {
                    let val = buffer[*local_idx];
                    let val_bytes = val.to_ne_bytes();
                    let is_fill = val_bytes.as_ref() == fill_bytes_slice;

                    if is_fill {
                        // Set NULL
                        value_vector.set_null(valid_rows + idx);
                    } else {
                        value_vector.as_mut_slice::<$rust_type>()[valid_rows + idx] = val;
                    }
                }
            }

            valid_rows += valid_coords.len();"""

# Second part: match arm for String
old_string_arm = """                    for i in 0..batch_size {
                        let local_idx = state.local_chunk_cursor + i;
                        let global_coords = crate::table_function::calculate_global_indices(
                            local_idx,
                            &bind_data.chunk_shape,
                            &state.current_chunk_grid,
                        );

                        let mut out_of_bounds = false;
                        for (dim, &global_coord) in global_coords.iter().enumerate().take(rank) {
                            if global_coord > bind_data.bounds_max[dim] {
                                out_of_bounds = true;
                                break;
                            }
                        }
                        if out_of_bounds {
                            continue;
                        }

                        for (dim, &global_coord) in global_coords.iter().enumerate().take(rank) {
                            if state.projected_columns.contains(&dim) {
                                if let Some(coord_vals) =
                                    bind_data.coords.get(&bind_data.dim_names[dim])
                                {
                                    let mut coord_vector = output.flat_vector(dim);
                                    let coord_slice = coord_vector.as_mut_slice::<f64>();
                                    coord_slice[valid_rows] = coord_vals
                                        .get(global_coord as usize)
                                        .copied()
                                        .unwrap_or(f64::NAN);
                                } else {
                                    let mut coord_vector = output.flat_vector(dim);
                                    let coord_slice = coord_vector.as_mut_slice::<i64>();
                                    coord_slice[valid_rows] = global_coord as i64;
                                }
                            }
                        }

                        if state.projected_columns.contains(&rank) {
                            let val = &buffer[local_idx];
                            // Insert string using the dedicated insert method
                            value_vector.insert(valid_rows, val.as_str());
                        }

                        valid_rows += 1;
                    }"""

new_string_arm = """                    let mut valid_coords = Vec::with_capacity(batch_size);
                    for i in 0..batch_size {
                        let local_idx = state.local_chunk_cursor + i;
                        let global_coords = crate::table_function::calculate_global_indices(
                            local_idx,
                            &bind_data.chunk_shape,
                            &state.current_chunk_grid,
                        );

                        let mut out_of_bounds = false;
                        for (dim, &global_coord) in global_coords.iter().enumerate().take(rank) {
                            if global_coord > bind_data.bounds_max[dim] {
                                out_of_bounds = true;
                                break;
                            }
                        }
                        if !out_of_bounds {
                            valid_coords.push((local_idx, global_coords));
                        }
                    }

                    for (dim, _) in (0..rank).enumerate() {
                        if state.projected_columns.contains(&dim) {
                            if let Some(coord_vals) = bind_data.coords.get(&bind_data.dim_names[dim]) {
                                let mut coord_vector = output.flat_vector(dim);
                                let coord_slice = coord_vector.as_mut_slice::<f64>();
                                for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                                    coord_slice[valid_rows + idx] = coord_vals
                                        .get(global_coords[dim] as usize)
                                        .copied()
                                        .unwrap_or(f64::NAN);
                                }
                            } else {
                                let mut coord_vector = output.flat_vector(dim);
                                let coord_slice = coord_vector.as_mut_slice::<i64>();
                                for (idx, (_, global_coords)) in valid_coords.iter().enumerate() {
                                    coord_slice[valid_rows + idx] = global_coords[dim] as i64;
                                }
                            }
                        }
                    }

                    if state.projected_columns.contains(&rank) {
                        for (idx, (local_idx, _)) in valid_coords.iter().enumerate() {
                            let val = &buffer[*local_idx];
                            // Insert string using the dedicated insert method
                            value_vector.insert(valid_rows + idx, val.as_str());
                        }
                    }

                    valid_rows += valid_coords.len();"""

content = content.replace(old_macro, new_macro)
content = content.replace(old_string_arm, new_string_arm)

with open('src/table_function.rs', 'w') as f:
    f.write(content)
