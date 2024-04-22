use std::{
    collections::{BTreeMap, HashMap, HashSet},
    convert::identity,
};

use anyhow::Result;
use image::{DynamicImage, Rgb};
use sixel_image::{Pixel, SixelColor, SixelImage};

const MAX_DEPTH: u8 = 8;

#[derive(Default, Debug, PartialEq)]
struct PixelNode {
    /// Index of the node in the tree.values vector
    id: PixelNodeId,
    /// Depth of the node in the tree
    depth: PixelNodeDepth,
    /// Parent node
    parent: Option<PixelNodeId>,
    /// List of direct children ids
    children: [Option<PixelNodeId>; 8],
    /// Total number of pixel reprensented in this cube (n1 in ImageMagick blog)
    total_count: usize,
    /// Number of pixel directly in the node (not reprensented in any child node) (n2 in
    /// ImageMagick blog)
    direct_count: usize,
    /// Average pixel value
    avg: [u8; 3],
    /// the distance squared in RGB space between each pixel contained within a node and the
    /// node's center. This represents the quantization error for a node.
    error: f64,
}

impl PixelNode {
    pub fn new(id: PixelNodeId, depth: PixelNodeDepth, parent: PixelNodeId) -> Self {
        Self {
            id,
            depth,
            parent: Some(parent),
            ..Default::default()
        }
    }

    pub fn new_root() -> Self {
        Self {
            id: PixelNodeId(0),
            depth: 0,
            parent: None,
            ..Default::default()
        }
    }

    pub fn child_id(pixel: &Rgb<u8>, depth: PixelNodeDepth) -> PixelNodeChildId {
        let index = MAX_DEPTH - depth;
        ((pixel.0[0] >> index & 0x1)
            | (pixel.0[1] >> index & 0x1) << 0x1
            | (pixel.0[2] >> index & 0x1) << 0x2) as usize
    }

    pub fn update_with(&mut self, pixel: &Rgb<u8>, depth: PixelNodeDepth, mid: &[f64; 3]) {
        self.total_count += 1;
        if depth == 8 {
            self.direct_count += 1;

            // Update average
            self.avg[0] -= (self.avg[0] as usize / self.total_count) as u8;
            self.avg[1] -= (self.avg[1] as usize / self.total_count) as u8;
            self.avg[2] -= (self.avg[2] as usize / self.total_count) as u8;
            self.avg[0] += (pixel.0[0] as usize / self.total_count) as u8;
            self.avg[1] += (pixel.0[1] as usize / self.total_count) as u8;
            self.avg[2] += (pixel.0[2] as usize / self.total_count) as u8;
        }

        // Distance
        let error = [
            (pixel.0[0] as f64 - mid[0]),
            (pixel.0[1] as f64 - mid[1]),
            (pixel.0[2] as f64 - mid[2]),
        ];

        let distance = error[0] * error[0] + error[1] * error[1] + error[2] * error[2];
        self.error += distance;
    }

    pub fn reset(&mut self) {
        self.total_count = 0;
        self.direct_count = 0;
        self.avg = [0, 0, 0];
        self.error = 0f64;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Ord, PartialOrd)]
struct PixelNodeId(usize);

/// Depth in the tree, 0 is root, 8 is leaf
type PixelNodeDepth = u8;
type PixelNodeChildId = usize;

#[derive(Debug)]
struct PixelTree {
    nodes: Vec<PixelNode>,
}

impl PixelTree {
    pub fn new() -> Self {
        Self {
            nodes: vec![PixelNode::new_root()],
        }
    }

    pub fn len(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.direct_count > 0)
            .count()
    }

    pub fn get_or_insert_child_mut<'a>(
        &'a mut self,
        parent: PixelNodeId,
        pixel: &Rgb<u8>,
        depth: PixelNodeDepth,
    ) -> &'a mut PixelNode {
        let child_id = PixelNode::child_id(pixel, depth);
        let parent_node = &self.nodes[parent.0];
        let current = if let Some(id) = parent_node.children[child_id] {
            id
        } else {
            let id = PixelNodeId(self.nodes.len());
            self.nodes.push(PixelNode::new(id, depth, parent));
            self.nodes[parent.0].children[child_id] = Some(id);
            id
        };

        &mut self.nodes[current.0]
    }

    pub fn insert(&mut self, pixel: Rgb<u8>) {
        let mut current_id = PixelNodeId(0);

        let root = &mut self.nodes[0];
        let mut mid: [f64; 3] = [128f64, 128f64, 128f64];
        let mut bisect = 128f64;

        root.update_with(&pixel, 0, &mid);

        for depth in 1..=8 {
            bisect *= 0.5;

            let child_id = PixelNode::child_id(&pixel, depth);

            mid[0] += if child_id & 1 == 0 { -bisect } else { bisect };
            mid[1] += if child_id & 2 == 0 { -bisect } else { bisect };
            mid[2] += if child_id & 4 == 0 { -bisect } else { bisect };

            let next_node = self.get_or_insert_child_mut(current_id, &pixel, depth);
            next_node.update_with(&pixel, depth, &mid);

            current_id = next_node.id;
        }
    }

    pub fn path<'a>(&'a self, pixel: &Rgb<u8>) -> PixelTreePath<'a> {
        PixelTreePath {
            depth: 0,
            next_id: Some(PixelNodeId(0)),
            tree: self,
            pixel: pixel.clone(),
        }
    }

    fn prune_node(&mut self, node_id: &PixelNodeId) {
        log::trace!("{:?}", self.get(node_id));
        // Prune children if needed
        let children = {
            let node = self.get(node_id);
            node.children
                .iter()
                .cloned()
                .filter_map(identity)
                .collect::<Vec<_>>()
        };
        for child_id in children {
            self.prune_node(&child_id);
        }

        // Propagate values to parent
        if self.get(node_id).direct_count > 0 {
            let (parent_id, direct_count, avg) = {
                let node = self.get(node_id);
                (node.parent.unwrap(), node.direct_count, node.avg)
            };

            let parent = self.get_mut(&parent_id);
            parent.direct_count += direct_count;

            parent.avg[0] -= (parent.avg[0] as usize * direct_count / parent.direct_count) as u8;
            parent.avg[1] -= (parent.avg[1] as usize * direct_count / parent.direct_count) as u8;
            parent.avg[2] -= (parent.avg[2] as usize * direct_count / parent.direct_count) as u8;

            parent.avg[0] += (avg[0] as usize * direct_count / parent.direct_count) as u8;
            parent.avg[1] += (avg[1] as usize * direct_count / parent.direct_count) as u8;
            parent.avg[2] += (avg[2] as usize * direct_count / parent.direct_count) as u8;
        }

        // Finally reset node values
        let node = self.get_mut(node_id);
        node.reset();
    }

    pub fn prune(&mut self, error_treshold: f64) {
        for node_id in (0..self.nodes.len()).map(PixelNodeId) {
            let (direct_count, error) = {
                let node = self.get(&node_id);
                (node.direct_count, node.error)
            };
            if direct_count > 0 && error >= error_treshold {
                self.prune_node(&node_id);
            }
        }
    }

    pub fn min_error(&mut self) -> f64 {
        self.nodes
            .iter()
            .filter(|node| node.direct_count > 0)
            .map(|node| node.error)
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap()
    }

    pub fn get<'a>(&'a self, id: &PixelNodeId) -> &'a PixelNode {
        &self.nodes[id.0]
    }

    pub fn get_mut<'a>(&'a mut self, id: &PixelNodeId) -> &'a mut PixelNode {
        &mut self.nodes[id.0]
    }
}

struct PixelTreePath<'a> {
    depth: PixelNodeDepth,
    next_id: Option<PixelNodeId>,
    tree: &'a PixelTree,
    pixel: Rgb<u8>,
}

impl<'a> Iterator for PixelTreePath<'a> {
    type Item = PixelNodeId;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_id {
            Some(current_id) => {
                if self.depth < MAX_DEPTH {
                    let current_node = self.tree.get(&current_id);
                    self.depth += 1;
                    self.next_id =
                        current_node.children[PixelNode::child_id(&self.pixel, self.depth)];
                } else {
                    self.next_id = None;
                }
                Some(current_id)
            }
            None => None,
        }
    }
}

/// Based on "Color Reduction Utilizing Adaptive Spatial Subdivision" https://imagemagick.org/script/quantize.php
pub fn adaptative_spatial_subdivision(
    pixels: Vec<Rgb<u8>>,
    palette_size: usize,
) -> (Vec<[u8; 3]>, Vec<usize>) {
    let mut tree = PixelTree::new();
    // Classification
    ass_classification(&mut tree, &pixels);
    // Reduction
    ass_reduction(&mut tree, palette_size);
    // Assignement
    ass_assignment(&tree, &pixels)
}

fn ass_classification(tree: &mut PixelTree, pixels: &[Rgb<u8>]) {
    for pixel in pixels {
        tree.insert(pixel.clone());
    }
}

fn ass_reduction(tree: &mut PixelTree, palette_size: usize) {
    let mut error_treshold = 0f64;

    while tree.len() > palette_size {
        log::debug!(
            "Tree len {} > palette_size {}, pruning up to {}",
            tree.len(),
            palette_size,
            error_treshold
        );
        tree.prune(error_treshold);
        error_treshold = tree.min_error();
    }

    log::info!("Final palette size: {}", tree.len())
}

fn ass_assignment(tree: &PixelTree, pixels: &[Rgb<u8>]) -> (Vec<[u8; 3]>, Vec<usize>) {
    let full_color_map = tree
        .nodes
        .iter()
        .filter(|node| node.direct_count > 0)
        .enumerate()
        .map(|(index, node)| (index, node.id, node.avg))
        .collect::<Vec<_>>();
    let colors = full_color_map
        .iter()
        .map(|(_, _, color)| color)
        .cloned()
        .collect();
    let color_map = full_color_map
        .into_iter()
        .map(|(index, node_id, _)| (node_id, index))
        .collect::<BTreeMap<_, _>>();
    let pixels = pixels
        .iter()
        .map(|pixel| {
            let node = tree
                .path(pixel)
                .filter(|node| tree.get(node).direct_count > 0)
                .last()
                .unwrap();
            *color_map.get(&node).unwrap()
        })
        .collect();

    (colors, pixels)
}

pub fn median_cut_quantization(pixels: Vec<Rgb<u8>>, palette_size: usize) -> Vec<SixelColor> {
    let mut bucket = pixels.clone();
    median_sort(&mut bucket, palette_size);
    let (_sixels, map) = median_quantization(&bucket, palette_size);

    pixels
        .iter()
        .map(|pixel| map.get(pixel).unwrap().clone())
        .collect()
}

fn median_sort(bucket: &mut [Rgb<u8>], depth: usize) {
    // Look for channel with widest range
    let r_max = bucket.iter().map(|pixel| pixel.0[0]).max().unwrap();
    let r_min = bucket.iter().map(|pixel| pixel.0[0]).min().unwrap();
    let g_max = bucket.iter().map(|pixel| pixel.0[1]).max().unwrap();
    let g_min = bucket.iter().map(|pixel| pixel.0[1]).min().unwrap();
    let b_max = bucket.iter().map(|pixel| pixel.0[2]).max().unwrap();
    let b_min = bucket.iter().map(|pixel| pixel.0[2]).min().unwrap();

    let widest_range = [r_max - r_min, g_max - g_min, b_max - b_min]
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.cmp(b))
        .map(|(index, _)| index)
        .unwrap();

    bucket.sort_by(|a, b| a.0[widest_range].cmp(&b.0[widest_range]));
    let median_index = bucket.len() / 2;

    if median_index > 0 && depth > 0 {
        median_sort(&mut bucket[..median_index], depth - 1);
        median_sort(&mut bucket[median_index..], depth - 1);
    }
}

fn median_quantization(
    pixels: &[Rgb<u8>],
    count: usize,
) -> (Vec<SixelColor>, HashMap<Rgb<u8>, SixelColor>) {
    let bucket_size = pixels.len() / count;
    let mut map: HashMap<Rgb<u8>, SixelColor> = HashMap::new();
    let sixels = pixels
        .chunks(bucket_size)
        .map(|bucket| {
            let real_bucket_size = bucket.len();
            let r = bucket
                .iter()
                .map(|pixel| pixel.0[0] as usize)
                .sum::<usize>()
                / real_bucket_size;
            let g = bucket
                .iter()
                .map(|pixel| pixel.0[1] as usize)
                .sum::<usize>()
                / real_bucket_size;
            let b = bucket
                .iter()
                .map(|pixel| pixel.0[2] as usize)
                .sum::<usize>()
                / real_bucket_size;

            let sixel = SixelColor::Rgb(
                (r * 100 / 256) as u8,
                (g * 100 / 256) as u8,
                (b * 100 / 256) as u8,
            );

            for pixel in bucket.iter() {
                map.insert(pixel.clone(), sixel.clone());
            }

            sixel
        })
        .collect();
    (sixels, map)
}

fn uniform_quantization_component(i: u8) -> u8 {
    // Scale from 0-255 to 0-100
    // but 0-100 rgb doesn't fit in u16
    // So we do uniform quantization by removing 3 lsb
    // Reducing by 2 lsb should add up correctly to fit u16 but it raises some issues
    // Limit seems to be at 1000 (tested on Alacritty and xterm), while Sixel spec limit it to 256
    (i as f32 * 100f32 / u8::MAX as f32) as u8 & 0b01111000
}

pub fn uniform_quantization(pixels: Vec<Rgb<u8>>) -> Vec<SixelColor> {
    pixels
        .iter()
        .map(|pixel| {
            let new_pixel = SixelColor::Rgb(
                uniform_quantization_component(pixel.0[0]),
                uniform_quantization_component(pixel.0[1]),
                uniform_quantization_component(pixel.0[2]),
            );

            new_pixel
        })
        .collect()
}

pub fn convert_to_sixel(image: DynamicImage) -> Result<SixelImage> {
    let palette_size = 999;
    let rgb8 = image.to_rgb8();
    let pixels: Vec<Rgb<u8>> = rgb8.pixels().cloned().collect();
    let (colors, pixels) = adaptative_spatial_subdivision(pixels, palette_size);

    Ok(SixelImage {
        color_registers: colors
            .into_iter()
            .enumerate()
            .map(|(index, pixel)| {
                (
                    index as u16,
                    SixelColor::Rgb(
                        (pixel[0] as u16 * 100 / 255) as u8,
                        (pixel[1] as u16 * 100 / 255) as u8,
                        (pixel[2] as u16 * 100 / 255) as u8,
                    ),
                )
            })
            .collect(),
        pixels: pixels
            .into_iter()
            .map(|index| Pixel {
                on: true,
                color: index as u16,
            })
            .collect::<Vec<Pixel>>()
            .chunks(image.width() as usize)
            .map(|line| line.iter().cloned().collect::<Vec<Pixel>>())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test]
    fn test_median_sort() {
        // Given
        let mut pixels = vec![
            Rgb::<u8>([255, 001, 254]),
            Rgb::<u8>([255, 253, 001]),
            Rgb::<u8>([001, 254, 001]),
            Rgb::<u8>([001, 001, 253]),
        ];

        // When
        median_sort(&mut pixels, 256);

        // Then
        assert_eq!(
            pixels,
            vec![
                Rgb::<u8>([001, 001, 253]),
                Rgb::<u8>([001, 254, 001]),
                Rgb::<u8>([255, 253, 001]),
                Rgb::<u8>([255, 001, 254]),
            ]
        );
    }

    #[test]
    fn test_median_quantization() {
        // Given
        let mut pixels = vec![
            Rgb::<u8>([255, 000, 100]),
            Rgb::<u8>([255, 010, 000]),
            Rgb::<u8>([001, 100, 000]),
            Rgb::<u8>([001, 000, 200]),
        ];

        // When
        let (sixels, _) = median_quantization(&mut pixels, 2);

        // Then
        assert_eq!(
            sixels,
            vec![SixelColor::Rgb(255, 5, 50), SixelColor::Rgb(1, 50, 100),]
        );
    }

    #[test]
    fn test_pixel_tree_path() {
        // Given
        let mut tree = PixelTree::new();
        let pixel = Rgb::<u8>([0x0, 0x0, 0x0]);
        tree.insert(pixel.clone());

        // When
        let ids = tree.path(&pixel).collect::<Vec<PixelNodeId>>();

        // Then
        assert_eq!(
            ids,
            vec![
                PixelNodeId(0),
                PixelNodeId(1),
                PixelNodeId(2),
                PixelNodeId(3),
                PixelNodeId(4),
                PixelNodeId(5),
                PixelNodeId(6),
                PixelNodeId(7),
                PixelNodeId(8)
            ]
        );
    }

    #[test]
    fn test_pixel_tree_insert() {
        // Given
        let mut tree = PixelTree::new();
        let r = 0b01101001;
        let g = 0b01011010;
        let b = 0b01110100;
        let pixel = Rgb::<u8>([r, g, b]);

        // When
        tree.insert(pixel.clone());

        // Then
        assert_eq!(
            tree.nodes,
            vec![
                PixelNode {
                    id: PixelNodeId(0),
                    depth: 0,
                    parent: None,
                    children: [
                        Some(PixelNodeId(1)),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None
                    ],
                    total_count: 1,
                    error: 2117.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(1),
                    depth: 1,
                    parent: Some(PixelNodeId(0)),
                    children: [
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(PixelNodeId(2))
                    ],
                    total_count: 1,
                    error: 5061.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(2),
                    depth: 2,
                    parent: Some(PixelNodeId(1)),
                    children: [
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(PixelNodeId(3)),
                        None,
                        None
                    ],
                    total_count: 1,
                    error: 517.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(3),
                    depth: 3,
                    parent: Some(PixelNodeId(2)),
                    children: [
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(PixelNodeId(4)),
                        None
                    ],
                    total_count: 1,
                    error: 165.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(4),
                    depth: 4,
                    parent: Some(PixelNodeId(3)),
                    children: [
                        None,
                        None,
                        None,
                        Some(PixelNodeId(5)),
                        None,
                        None,
                        None,
                        None
                    ],
                    total_count: 1,
                    error: 21.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(5),
                    depth: 5,
                    parent: Some(PixelNodeId(4)),
                    children: [
                        None,
                        None,
                        None,
                        None,
                        Some(PixelNodeId(6)),
                        None,
                        None,
                        None
                    ],
                    total_count: 1,
                    error: 13.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(6),
                    depth: 6,
                    parent: Some(PixelNodeId(5)),
                    children: [
                        None,
                        None,
                        Some(PixelNodeId(7)),
                        None,
                        None,
                        None,
                        None,
                        None
                    ],
                    total_count: 1,
                    error: 5.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(7),
                    depth: 7,
                    parent: Some(PixelNodeId(6)),
                    children: [
                        None,
                        Some(PixelNodeId(8)),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None
                    ],
                    total_count: 1,
                    error: 2.0,
                    ..Default::default()
                },
                PixelNode {
                    id: PixelNodeId(8),
                    depth: 8,
                    parent: Some(PixelNodeId(7)),
                    children: [None, None, None, None, None, None, None, None],
                    total_count: 1,
                    direct_count: 1,
                    avg: pixel.0.clone(),
                    error: 0.75,
                    ..Default::default()
                },
            ]
        );
    }

    #[test]
    fn test_pixel_child_id() {
        // Given
        let pixel = Rgb::<u8>([0b01101001, 0b01011010, 0b01110100]);

        // Then
        assert_eq!(PixelNode::child_id(&pixel, 8), 0b001);
        assert_eq!(PixelNode::child_id(&pixel, 7), 0b010);
        assert_eq!(PixelNode::child_id(&pixel, 6), 0b100);
        assert_eq!(PixelNode::child_id(&pixel, 5), 0b011);
        assert_eq!(PixelNode::child_id(&pixel, 4), 0b110);
        assert_eq!(PixelNode::child_id(&pixel, 3), 0b101);
        assert_eq!(PixelNode::child_id(&pixel, 2), 0b111);
        assert_eq!(PixelNode::child_id(&pixel, 1), 0b000);
    }

    #[test]
    fn test_pixel_tree_count() {
        // Given
        let mut tree = PixelTree::new();

        // When
        tree.insert(Rgb::<u8>([0x00, 0x00, 0x00]));
        tree.insert(Rgb::<u8>([0x01, 0x01, 0x01]));
        tree.insert(Rgb::<u8>([0x01, 0x01, 0x01]));
        let len = tree.len();

        // Then
        assert_eq!(len, 2);
    }

    #[test]
    fn test_pixel_tree_avg_and_error() {
        // Given
        let mut tree = PixelTree::new();
        let pixel_a = Rgb::<u8>([0x00, 0x00, 0b100]);
        let pixel_b = Rgb::<u8>([0x00, 0x00, 0b000]);
        tree.insert(pixel_a);
        tree.insert(pixel_b);

        // When
        let first_divergent_node_for_pixel_a = tree.path(&pixel_a).nth(6).unwrap();
        tree.prune_node(&first_divergent_node_for_pixel_a);
        let first_divergent_node_for_pixel_b = tree.path(&pixel_b).nth(6).unwrap();
        tree.prune_node(&first_divergent_node_for_pixel_b);

        // Then
        let node_id = tree.path(&pixel_a).nth(5).unwrap();
        let node = tree.get(&node_id);

        assert_eq!(node.depth, 5);
        assert_eq!(
            node.children
                .iter()
                .filter_map(Option::as_ref)
                .collect::<Vec<_>>()
                .len(),
            2
        );
        assert_eq!(node.avg, [0x0, 0x0, 0b010]);
        assert_eq!(node.error, (4f64 * 4f64) * 5f64);
    }
}
