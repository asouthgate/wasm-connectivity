export const PRESETS = [
  {
    group: 'Pairwise · 200×200', mode: 'pairwise',
    items: [
      { id:'u200', name:'uniform',
        res:'test_grids/uniform_200_res.asc', pts:'test_grids/uniform_200_pts.asc' },
      { id:'t200', name:'two corridors',
        res:'test_grids/two_paths_200_res.asc', pts:'test_grids/two_paths_200_pts.asc' },
      { id:'r200', name:'random terrain',
        res:'test_grids/rand_terrain_200_res.asc', pts:'test_grids/rand_terrain_200_pts.asc' },
    ],
  },
  {
    group: 'Pairwise · 300×300', mode: 'pairwise',
    items: [
      { id:'u300', name:'uniform',
        res:'test_grids/uniform_300_res.asc', pts:'test_grids/uniform_300_pts.asc' },
      { id:'t300', name:'two corridors',
        res:'test_grids/two_paths_300_res.asc', pts:'test_grids/two_paths_300_pts.asc' },
      { id:'r300', name:'random terrain',
        res:'test_grids/rand_terrain_300_res.asc', pts:'test_grids/rand_terrain_300_pts.asc' },
    ],
  },
  {
    group: 'Pairwise · 500×500', mode: 'pairwise',
    items: [
      { id:'u500', name:'uniform',
        res:'test_grids/uniform_500_res.asc', pts:'test_grids/uniform_500_pts.asc' },
      { id:'t500', name:'two corridors',
        res:'test_grids/two_paths_500_res.asc', pts:'test_grids/two_paths_500_pts.asc' },
      { id:'r500', name:'random terrain',
        res:'test_grids/rand_terrain_500_res.asc', pts:'test_grids/rand_terrain_500_pts.asc' },
    ],
  },
  {
    group: 'Raster · 200×200', mode: 'raster',
    items: [
      { id:'ae200', name:'edge to edge',
        res:'test_grids/uniform_200_res.asc', src:'test_grids/adv_edge_src_200.asc', gnd:'test_grids/adv_edge_gnd_200.asc' },
      { id:'ac200', name:'center to ring',
        res:'test_grids/uniform_200_res.asc', src:'test_grids/adv_center_src_200.asc', gnd:'test_grids/adv_ring_gnd_200.asc' },
      { id:'ap200', name:'two patches',
        res:'test_grids/rand_terrain_200_res.asc', src:'test_grids/adv_patch_src_200.asc', gnd:'test_grids/adv_patch_gnd_200.asc' },
    ],
  },
  {
    group: 'Raster · 300×300', mode: 'raster',
    items: [
      { id:'ae300', name:'edge to edge',
        res:'test_grids/uniform_300_res.asc', src:'test_grids/adv_edge_src_300.asc', gnd:'test_grids/adv_edge_gnd_300.asc' },
      { id:'ac300', name:'center to ring',
        res:'test_grids/uniform_300_res.asc', src:'test_grids/adv_center_src_300.asc', gnd:'test_grids/adv_ring_gnd_300.asc' },
      { id:'ap300', name:'two patches',
        res:'test_grids/rand_terrain_300_res.asc', src:'test_grids/adv_patch_src_300.asc', gnd:'test_grids/adv_patch_gnd_300.asc' },
    ],
  },
  {
    group: 'Raster · 500×500', mode: 'raster',
    items: [
      { id:'ae500', name:'edge to edge',
        res:'test_grids/uniform_500_res.asc', src:'test_grids/adv_edge_src_500.asc', gnd:'test_grids/adv_edge_gnd_500.asc' },
      { id:'ac500', name:'center to ring',
        res:'test_grids/uniform_500_res.asc', src:'test_grids/adv_center_src_500.asc', gnd:'test_grids/adv_ring_gnd_500.asc' },
      { id:'ap500', name:'two patches',
        res:'test_grids/rand_terrain_500_res.asc', src:'test_grids/adv_patch_src_500.asc', gnd:'test_grids/adv_patch_gnd_500.asc' },
    ],
  },
];
