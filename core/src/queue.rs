use std::path::PathBuf;
use rand::seq::SliceRandom;
use rand::rngs::ThreadRng;

pub struct WallpaperQueue {
    all: Vec<PathBuf>,
    remaining: Vec<PathBuf>,
}

impl WallpaperQueue {
    pub fn new(images: Vec<PathBuf>) -> Self {
        let mut remaining = images.clone();
        remaining.shuffle(&mut ThreadRng::default());
        WallpaperQueue { all: images, remaining }
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// The full set of images this queue was built from, in the order it was given.
    /// Callers use it to detect that the folder's contents have drifted from the queue.
    pub fn all(&self) -> &[PathBuf] {
        &self.all
    }

    pub fn next(&mut self) -> Option<PathBuf> {
        if self.all.is_empty() {
            return None;
        }
        if self.remaining.is_empty() {
            self.remaining = self.all.clone();
            self.remaining.shuffle(&mut ThreadRng::default());
        }
        self.remaining.pop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn images(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("/wp/{i}.png"))).collect()
    }

    #[test]
    fn empty_queue_always_returns_none() {
        let mut queue = WallpaperQueue::new(vec![]);
        assert!(queue.is_empty());
        assert_eq!(queue.next(), None);
        assert_eq!(queue.next(), None);
    }

    #[test]
    fn every_image_appears_exactly_once_before_any_repeat() {
        let all = images(5);
        let mut queue = WallpaperQueue::new(all.clone());

        let mut seen = HashSet::new();
        for _ in 0..all.len() {
            let picked = queue.next().expect("queue should not be empty yet");
            assert!(seen.insert(picked), "image repeated before the folder was exhausted");
        }
        assert_eq!(seen.len(), all.len());
    }

    #[test]
    fn queue_reshuffles_and_keeps_producing_after_exhaustion() {
        let all = images(3);
        let mut queue = WallpaperQueue::new(all.clone());

        for _ in 0..all.len() {
            queue.next();
        }
        // one more pull past exhaustion must still yield a valid image, not None
        let picked = queue.next().expect("queue should reshuffle after exhaustion");
        assert!(all.contains(&picked));
    }

    #[test]
    fn single_image_folder_keeps_returning_the_same_image() {
        let all = images(1);
        let mut queue = WallpaperQueue::new(all.clone());

        assert_eq!(queue.next(), Some(all[0].clone()));
        assert_eq!(queue.next(), Some(all[0].clone()));
    }
}
