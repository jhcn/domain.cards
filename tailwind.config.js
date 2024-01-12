/** @type {import('tailwindcss').Config} */
module.exports = {
  mode: 'jit',
  content: ['./templates/*.{html,js}'],
  theme: {
    extend: {
      fontFamily: {
        ubuntu: ['ubuntu'],
        code: ['Courier New']
      },
      boxShadow: {
        bento: '0 2px 4px 0 rgb(0 0 0/.04)',
        avatar: '0 1px 2px 0 rgb(0 0 0/.1)'
      },
      borderRadius: {
        bento: '1.25rem'
      }
    },
    screens: {
      '2xl': {'max': '1535px'},
      // => @media (max-width: 1535px) { ... }

      'xl': {'max': '1279px'},
      // => @media (max-width: 1279px) { ... }

      'lg': {'max': '1023px'},
      // => @media (max-width: 1023px) { ... }

      'md': {'max': '767px'},
      // => @media (max-width: 767px) { ... }

      'sm': {'max': '639px'},
      // => @media (max-width: 639px) { ... }
    },
  },
  plugins: [],
}

