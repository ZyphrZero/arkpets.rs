use eframe::egui::{Pos2, Rect, Vec2};

const EPSILON: f32 = 0.001;

/// Cover every vertical strip, so gaps between staggered displays stay solid.
pub fn covered(rect: Rect, areas: &[Rect]) -> bool {
    if areas.iter().any(|area| area.contains_rect(rect)) {
        return true;
    }
    let mut xs = vec![rect.left(), rect.right()];
    for area in areas {
        for x in [area.left(), area.right()] {
            if x > rect.left() && x < rect.right() {
                xs.push(x);
            }
        }
    }
    xs.sort_by(f32::total_cmp);
    xs.windows(2).all(|strip| {
        let x = (strip[0] + strip[1]) * 0.5;
        let mut intervals: Vec<_> = areas
            .iter()
            .filter(|area| x >= area.left() && x <= area.right())
            .map(|area| (area.top(), area.bottom()))
            .collect();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut bottom = rect.top();
        for (start, end) in intervals {
            if start > bottom + EPSILON {
                break;
            }
            bottom = bottom.max(end);
        }
        bottom + EPSILON >= rect.bottom()
    })
}

/// Find the closest valid placement, used for deployment, dragging and hotplug.
pub fn project(position: Pos2, size: Vec2, areas: &[Rect]) -> Option<Pos2> {
    if covered(Rect::from_min_size(position, size), areas) {
        return Some(position);
    }
    let mut xs = vec![position.x];
    let mut ys = vec![position.y];
    for area in areas {
        for edge in [area.left(), area.right()] {
            xs.extend([edge, edge - size.x]);
        }
        for edge in [area.top(), area.bottom()] {
            ys.extend([edge, edge - size.y]);
        }
    }
    xs.iter()
        .flat_map(|&x| ys.iter().map(move |&y| Pos2::new(x, y)))
        .filter(|&p| covered(Rect::from_min_size(p, size), areas))
        .min_by(|a, b| a.distance_sq(position).total_cmp(&b.distance_sq(position)))
}

/// Sweep each axis through the union; even a fast throw cannot jump a gap.
pub fn travel(from: Pos2, to: Pos2, size: Vec2, areas: &[Rect]) -> Pos2 {
    fn axis(from: Pos2, to: Pos2, size: Vec2, areas: &[Rect]) -> Pos2 {
        let valid = |p| {
            covered(
                Rect::from_min_size(from, size).union(Rect::from_min_size(p, size)),
                areas,
            )
        };
        if valid(to) {
            return to;
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..24 {
            let middle = (low + high) * 0.5;
            if valid(from + (to - from) * middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        from + (to - from) * low
    }
    let horizontal = axis(from, Pos2::new(to.x, from.y), size, areas);
    axis(horizontal, Pos2::new(horizontal.x, to.y), size, areas)
}

/// Plan a shortest axis-aligned route around gaps in the screen union.
/// The result excludes `from` and includes `to`; an unobstructed rectangle
/// needs only the final target, allowing a direct diagonal transition.
pub fn waypoints(from: Pos2, to: Pos2, size: Vec2, areas: &[Rect]) -> Option<Vec<Pos2>> {
    if !from.is_finite()
        || !to.is_finite()
        || !size.is_finite()
        || size.x <= 0.0
        || size.y <= 0.0
        || areas.is_empty()
        || areas.iter().any(|area| {
            !area.min.is_finite()
                || !area.max.is_finite()
                || area.width() <= 0.0
                || area.height() <= 0.0
        })
    {
        return None;
    }
    let swept = |a: Pos2, b: Pos2| {
        covered(
            Rect::from_min_size(a, size).union(Rect::from_min_size(b, size)),
            areas,
        )
    };
    if !swept(from, from) || !swept(to, to) {
        return None;
    }
    if swept(from, to) {
        return Some(vec![to]);
    }

    // These coordinates contain every corner where a window-sized rectangle
    // can turn around a monitor edge, including placements spanning two screens.
    let mut xs = vec![from.x, to.x];
    let mut ys = vec![from.y, to.y];
    for area in areas {
        for edge in [area.left(), area.right()] {
            xs.extend([edge, edge - size.x]);
        }
        for edge in [area.top(), area.bottom()] {
            ys.extend([edge, edge - size.y]);
        }
    }
    xs.sort_by(f32::total_cmp);
    xs.dedup();
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let mut points = Vec::new();
    let mut grid = vec![None; xs.len() * ys.len()];
    for (y_index, &y) in ys.iter().enumerate() {
        for (x_index, &x) in xs.iter().enumerate() {
            let point = Pos2::new(x, y);
            if swept(point, point) {
                grid[y_index * xs.len() + x_index] = Some(points.len());
                points.push(point);
            }
        }
    }
    let source = points.iter().position(|&point| point == from)?;
    let target = points.iter().position(|&point| point == to)?;
    let mut edges = vec![Vec::new(); points.len()];
    for (index, &node) in grid.iter().enumerate() {
        let Some(node) = node else { continue };
        let neighbors = [
            (index % xs.len() > 0).then(|| grid[index - 1]).flatten(),
            (index >= xs.len())
                .then(|| grid[index - xs.len()])
                .flatten(),
        ];
        for neighbor in neighbors.into_iter().flatten() {
            if swept(points[node], points[neighbor]) {
                let a = points[node];
                let b = points[neighbor];
                let distance = (f64::from(a.x) - f64::from(b.x)).abs()
                    + (f64::from(a.y) - f64::from(b.y)).abs();
                edges[node].push((neighbor, distance));
                edges[neighbor].push((node, distance));
            }
        }
    }

    let mut distances = vec![f64::INFINITY; points.len()];
    let mut previous = vec![None; points.len()];
    let mut visited = vec![false; points.len()];
    distances[source] = 0.0;
    loop {
        let current = (0..points.len())
            .filter(|&index| !visited[index])
            .min_by(|&a, &b| distances[a].total_cmp(&distances[b]))?;
        if !distances[current].is_finite() {
            return None;
        }
        if current == target {
            break;
        }
        visited[current] = true;
        for &(neighbor, length) in &edges[current] {
            let distance = distances[current] + length;
            if distance < distances[neighbor] {
                distances[neighbor] = distance;
                previous[neighbor] = Some(current);
            }
        }
    }

    let mut reversed = Vec::new();
    let mut current = target;
    while current != source {
        reversed.push(points[current]);
        current = previous[current]?;
    }
    let mut path = vec![from];
    for point in reversed.into_iter().rev() {
        if path.len() >= 2 {
            let a = path[path.len() - 2];
            let b = path[path.len() - 1];
            if (a.x == b.x && b.x == point.x) || (a.y == b.y && b.y == point.y) {
                path.pop();
            }
        }
        path.push(point);
    }
    path.remove(0);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    #[test]
    fn adjacent_displays_allow_seam_crossing_but_staggered_gaps_do_not() {
        let size = Vec2::new(100.0, 100.0);
        let areas = [
            area(-1000.0, 0.0, 1000.0, 800.0),
            area(0.0, 200.0, 1200.0, 800.0),
        ];
        assert_eq!(
            travel(
                Pos2::new(-150.0, 400.0),
                Pos2::new(50.0, 400.0),
                size,
                &areas
            ),
            Pos2::new(50.0, 400.0)
        );
        let blocked = travel(Pos2::new(-150.0, 0.0), Pos2::new(50.0, 0.0), size, &areas);
        assert!((blocked.x + 100.0).abs() < 0.01);
        assert!(covered(Rect::from_min_size(blocked, size), &areas));
        let gap = [area(0.0, 0.0, 500.0, 500.0), area(600.0, 0.0, 500.0, 500.0)];
        assert!(travel(Pos2::new(350.0, 300.0), Pos2::new(800.0, 300.0), size, &gap).x < 401.0);
    }

    #[test]
    fn projection_recovers_removed_displays_and_never_selects_empty_space() {
        let areas = [
            area(-800.0, -600.0, 800.0, 600.0),
            area(0.0, 0.0, 1200.0, 800.0),
        ];
        let size = Vec2::new(200.0, 150.0);
        for p in [
            Pos2::new(-400.0, 200.0),
            Pos2::new(100.0, -300.0),
            Pos2::new(-2000.0, -1000.0),
        ] {
            let projected = project(p, size, &areas).unwrap();
            assert!(covered(Rect::from_min_size(projected, size), &areas));
            assert!(covered(
                Rect::from_min_size(project(projected, size, &areas[1..]).unwrap(), size),
                &areas[1..]
            ));
        }
        assert!(project(Pos2::ZERO, Vec2::splat(4000.0), &areas).is_none());
    }

    fn assert_route(from: Pos2, to: Pos2, size: Vec2, areas: &[Rect], route: &[Pos2]) -> f32 {
        assert_eq!(route.last(), Some(&to));
        assert!(!route.contains(&from));
        let mut previous = from;
        let mut distance = 0.0;
        for &point in route {
            assert!(covered(
                Rect::from_min_size(previous, size).union(Rect::from_min_size(point, size)),
                areas,
            ));
            assert!(travel(previous, point, size, areas).distance(point) < 0.01);
            distance += previous.distance(point);
            previous = point;
        }
        distance
    }

    #[test]
    fn unobstructed_route_has_only_the_target() {
        let areas = [area(0.0, 0.0, 1000.0, 800.0)];
        let from = Pos2::new(10.0, 20.0);
        let to = Pos2::new(400.0, 600.0);
        let size = Vec2::splat(100.0);
        let route = waypoints(from, to, size, &areas).unwrap();
        assert_eq!(route, vec![to]);
        assert_route(from, to, size, &areas, &route);
    }

    #[test]
    fn l_shaped_route_turns_without_crossing_the_inner_corner() {
        let areas = [
            area(0.0, 0.0, 400.0, 600.0),
            area(400.0, 400.0, 400.0, 600.0),
        ];
        let size = Vec2::splat(100.0);
        for (from, to) in [
            (Pos2::new(100.0, 100.0), Pos2::new(600.0, 800.0)),
            (Pos2::new(600.0, 800.0), Pos2::new(100.0, 100.0)),
        ] {
            let route = waypoints(from, to, size, &areas).unwrap();
            assert!(route.len() > 1);
            assert_eq!(assert_route(from, to, size, &areas, &route), 1200.0);
        }
    }

    #[test]
    fn u_shaped_route_uses_the_shortest_top_corridor_and_compresses_straight_runs() {
        let areas = [
            area(0.0, 0.0, 400.0, 1000.0),
            area(400.0, 0.0, 400.0, 300.0),
            area(800.0, 0.0, 400.0, 1000.0),
        ];
        let size = Vec2::splat(100.0);
        let from = Pos2::new(300.0, 600.0);
        let to = Pos2::new(800.0, 600.0);
        assert!(travel(from, to, size, &areas).distance(from) < 0.01);
        let route = waypoints(from, to, size, &areas).unwrap();
        assert_eq!(
            route,
            vec![Pos2::new(300.0, 200.0), Pos2::new(800.0, 200.0), to]
        );
        assert_eq!(assert_route(from, to, size, &areas, &route), 1300.0);
    }

    #[test]
    fn route_around_a_gap_supports_negative_screen_coordinates() {
        let areas = [
            area(-1200.0, -800.0, 400.0, 1000.0),
            area(-800.0, -800.0, 400.0, 300.0),
            area(-400.0, -800.0, 400.0, 1000.0),
        ];
        let from = Pos2::new(-900.0, -200.0);
        let to = Pos2::new(-400.0, -200.0);
        let size = Vec2::splat(100.0);
        let route = waypoints(from, to, size, &areas).unwrap();
        assert_eq!(assert_route(from, to, size, &areas, &route), 1300.0);
    }

    #[test]
    fn disconnected_screens_and_too_narrow_corridors_have_no_route() {
        let mut areas = vec![
            area(0.0, 0.0, 400.0, 1000.0),
            area(800.0, 0.0, 400.0, 1000.0),
        ];
        let from = Pos2::new(300.0, 600.0);
        let to = Pos2::new(800.0, 600.0);
        let size = Vec2::splat(100.0);
        assert_eq!(waypoints(from, to, size, &areas), None);
        areas.push(area(400.0, 0.0, 400.0, 99.5));
        assert_eq!(waypoints(from, to, size, &areas), None);
    }

    #[test]
    fn route_requires_valid_endpoints_and_a_window_that_fits() {
        let areas = [area(0.0, 0.0, 0.5, 0.5)];
        let from = Pos2::ZERO;
        let to = Pos2::new(0.25, 0.25);
        let size = Vec2::splat(0.1);
        assert_eq!(waypoints(from, to, size, &areas), Some(vec![to]));
        assert_eq!(waypoints(from, to, Vec2::splat(0.6), &areas), None);
        assert_eq!(waypoints(from, to, Vec2::ZERO, &areas), None);
        assert_eq!(waypoints(Pos2::new(-1.0, 0.0), to, size, &areas), None);
        assert_eq!(waypoints(from, Pos2::new(1.0, 0.0), size, &areas), None);
        assert_eq!(waypoints(from, to, size, &[]), None);
        assert_eq!(waypoints(from, to, Vec2::splat(f32::NAN), &areas), None);
    }
}
