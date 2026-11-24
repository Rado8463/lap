// src/router.js
import { createRouter, createWebHistory } from 'vue-router';
import { standaloneViewer } from '@/common/standalone';

const routes = [
  {
    path: '/',
    name: 'Home',
    component: () => import('@/views/Home.vue'),
  },
  {
    path: '/image-viewer',
    name: 'ImageViewer',
    component: () => import('@/views/ImageViewer.vue'),
  },
  {
    path: '/image-editor',
    name: 'ImageEditor',
    component: () => import('@/views/ImageEditor.vue'),
  },
  {
    path: '/montage',
    name: 'Montage',
    component: () => import('@/views/Montage.vue'),
  },
  {
    path: '/info-window',
    name: 'InfoWindow',
    component: () => import('@/views/InfoWindow.vue'),
  },
  {
    path: '/settings',
    name: 'Settings',
    component: () => import('@/views/Settings.vue'),
  }
];

const router = createRouter({
  history: createWebHistory(),
  routes,
});

// Lap started with an image path (Explorer "Open with"): show the Image Viewer instead of Home.
router.beforeEach((to) => {
  if (to.name === 'Home' && standaloneViewer.value) {
    return { path: '/image-viewer', query: { standalone: '1' }, replace: true };
  }
  return true;
});

export default router;
